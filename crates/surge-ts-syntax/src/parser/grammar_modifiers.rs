//! tsc's `checkGrammarModifiers`: the modifier keywords written on a
//! declaration, judged in order, stopping at the first one it rejects. oxc
//! drops the modifiers it does not accept from the AST (reporting them as
//! parse errors of its own), so they are read back from the source text
//! between a declaration's start and its keyword or name.
//!
//! The whole decision table is ported so that a code this pass reports is
//! only reported when tsc would reach it; the caller decides which codes it
//! owns.

use oxc_ast::ast::{Program, Statement};
use oxc_span::{GetSpan, Span};

use super::spans::text_span_from_oxc_span;
use crate::{ParsedGrammarDiagnostic, ParsedGrammarDiagnosticKind as Kind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ModifierKind {
    Public,
    Private,
    Protected,
    Static,
    Readonly,
    Abstract,
    Override,
    Declare,
    Async,
    Accessor,
    Const,
    Export,
    Default,
    In,
    Out,
}

impl ModifierKind {
    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "public" => Self::Public,
            "private" => Self::Private,
            "protected" => Self::Protected,
            "static" => Self::Static,
            "readonly" => Self::Readonly,
            "abstract" => Self::Abstract,
            "override" => Self::Override,
            "declare" => Self::Declare,
            "async" => Self::Async,
            "accessor" => Self::Accessor,
            "const" => Self::Const,
            "export" => Self::Export,
            "default" => Self::Default,
            "in" => Self::In,
            "out" => Self::Out,
            _ => return None,
        })
    }

    pub(super) fn text(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Protected => "protected",
            Self::Static => "static",
            Self::Readonly => "readonly",
            Self::Abstract => "abstract",
            Self::Override => "override",
            Self::Declare => "declare",
            Self::Async => "async",
            Self::Accessor => "accessor",
            Self::Const => "const",
            Self::Export => "export",
            Self::Default => "default",
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Modifier {
    pub kind: ModifierKind,
    pub span: Span,
    /// A JSDoc modifier tag (`NodeFlagsReparsed`): it has no written position,
    /// so the "must precede" order rules skip it.
    pub reparsed: bool,
}

/// The modifier keywords written from `start` on, in order, up to the first
/// word that is not one (the declaration keyword, a name, `get`, `*`, `[`).
/// `const` counts only when `const_is_modifier`, since on a variable
/// statement it is the declaration keyword. A decorator in the range makes
/// the list unreliable, and `None` is returned.
pub(super) fn scan_modifiers(
    source_text: &str,
    start: u32,
    end: u32,
    const_is_modifier: bool,
) -> Option<Vec<Modifier>> {
    let text = source_text.get(start as usize..end as usize)?;
    let mut modifiers = Vec::new();
    let mut offset = 0usize;
    loop {
        let rest = &text[offset..];
        let trimmed = rest.trim_start();
        offset += rest.len() - trimmed.len();
        if trimmed.is_empty() {
            break;
        }
        if let Some(comment) = trimmed.strip_prefix("//") {
            let line_len = comment.find('\n').map_or(comment.len(), |at| at + 1);
            offset += 2 + line_len;
            continue;
        }
        if let Some(comment) = trimmed.strip_prefix("/*") {
            let Some(close) = comment.find("*/") else { break };
            offset += 2 + close + 2;
            continue;
        }
        if trimmed.starts_with('@') {
            return None;
        }
        let word_len = trimmed
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .unwrap_or(trimmed.len());
        if word_len == 0 {
            break;
        }
        let word = &trimmed[..word_len];
        let Some(kind) = ModifierKind::parse(word) else { break };
        if kind == ModifierKind::Const && !const_is_modifier {
            break;
        }
        let word_start = start + offset as u32;
        modifiers.push(Modifier {
            kind,
            span: Span::new(word_start, word_start + word_len as u32),
            reparsed: false,
        });
        offset += word_len;
    }
    Some(modifiers)
}

/// The tsc node kinds `checkGrammarModifiers` distinguishes, for the nodes
/// this pass scans.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NodeKind {
    ClassDeclaration,
    FunctionDeclaration,
    VariableStatement,
    InterfaceDeclaration,
    TypeAliasDeclaration,
    EnumDeclaration,
    ModuleDeclaration,
    PropertyDeclaration,
    MethodDeclaration,
    GetAccessor,
    SetAccessor,
    Constructor,
    TypeParameter,
    Parameter,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Parent {
    SourceFile,
    /// A namespace or module body; `namespace` when the enclosing declaration
    /// is a non-ambient module (an identifier-named one).
    ModuleBlock { namespace: bool },
    Class { is_declaration: bool, is_abstract: bool },
    Other,
}

/// What owns a type parameter list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TypeParameterOwner {
    FunctionLike,
    Class,
    Interface,
    TypeAlias,
    Other,
}

pub(super) struct ModifierContext {
    pub node: NodeKind,
    pub parent: Parent,
    /// tsc's `node.Parent.Flags & NodeFlagsAmbient`.
    pub parent_ambient: bool,
    pub name_is_private: bool,
    pub type_parameter_owner: TypeParameterOwner,
}

pub(super) struct ModifierError {
    pub code: u32,
    pub span: Span,
    pub args: Vec<&'static str>,
}

fn error(code: u32, span: Span, args: &[&'static str]) -> Option<ModifierError> {
    Some(ModifierError { code, span, args: args.to_vec() })
}

const FLAG_EXPORT: u32 = 1 << 0;
const FLAG_AMBIENT: u32 = 1 << 1;
const FLAG_PUBLIC: u32 = 1 << 2;
const FLAG_PRIVATE: u32 = 1 << 3;
const FLAG_PROTECTED: u32 = 1 << 4;
const FLAG_STATIC: u32 = 1 << 5;
const FLAG_READONLY: u32 = 1 << 6;
const FLAG_ACCESSOR: u32 = 1 << 7;
const FLAG_ABSTRACT: u32 = 1 << 8;
const FLAG_ASYNC: u32 = 1 << 9;
const FLAG_DEFAULT: u32 = 1 << 10;
const FLAG_OVERRIDE: u32 = 1 << 11;
const FLAG_IN: u32 = 1 << 12;
const FLAG_OUT: u32 = 1 << 13;
const FLAG_ACCESSIBILITY: u32 = FLAG_PUBLIC | FLAG_PRIVATE | FLAG_PROTECTED;

fn accessibility_flag(kind: ModifierKind) -> u32 {
    match kind {
        ModifierKind::Public => FLAG_PUBLIC,
        ModifierKind::Private => FLAG_PRIVATE,
        _ => FLAG_PROTECTED,
    }
}

fn is_class_member(node: NodeKind) -> bool {
    matches!(
        node,
        NodeKind::PropertyDeclaration
            | NodeKind::MethodDeclaration
            | NodeKind::GetAccessor
            | NodeKind::SetAccessor
            | NodeKind::Constructor
    )
}

/// tsc's `findFirstIllegalModifier`: a declaration that is not a module
/// element takes no modifiers at all (TS1184), except the ones that are part
/// of its own syntax.
fn first_illegal_modifier(modifiers: &[Modifier], ctx: &ModifierContext) -> Option<Modifier> {
    if is_class_member(ctx.node)
        || matches!(
            ctx.node,
            NodeKind::ModuleDeclaration | NodeKind::TypeParameter | NodeKind::Parameter
        )
    {
        return None;
    }
    if matches!(ctx.parent, Parent::SourceFile | Parent::ModuleBlock { .. }) {
        return None;
    }
    let allowed = match ctx.node {
        NodeKind::FunctionDeclaration => Some(ModifierKind::Async),
        NodeKind::ClassDeclaration => Some(ModifierKind::Abstract),
        NodeKind::EnumDeclaration => Some(ModifierKind::Const),
        _ => None,
    };
    modifiers.iter().find(|modifier| Some(modifier.kind) != allowed).copied()
}

/// The first modifier error tsc reports on a declaration, if any. Codes
/// other pass or oxc already report come back too, so the caller can tell
/// that its own code would not have been reached.
pub(super) fn first_modifier_error(
    modifiers: &[Modifier],
    ctx: &ModifierContext,
) -> Option<ModifierError> {
    if modifiers.is_empty() {
        return None;
    }
    if let Some(modifier) = first_illegal_modifier(modifiers, ctx) {
        return error(1184, modifier.span, &[]);
    }
    let in_class = matches!(ctx.parent, Parent::Class { .. });
    let at_module_level = matches!(ctx.parent, Parent::SourceFile | Parent::ModuleBlock { .. });
    let mut flags = 0u32;
    let mut last_async: Option<Modifier> = None;
    let mut last_static: Option<Modifier> = None;
    let mut last_override: Option<Modifier> = None;
    for &modifier in modifiers {
        let span = modifier.span;
        let text = modifier.kind.text();
        if !matches!(modifier.kind, ModifierKind::In | ModifierKind::Out | ModifierKind::Const)
            && ctx.node == NodeKind::TypeParameter
        {
            return error(1273, span, &[text]);
        }
        match modifier.kind {
            ModifierKind::Const => {
                if !matches!(ctx.node, NodeKind::EnumDeclaration | NodeKind::TypeParameter) {
                    return error(1248, span, &["const"]);
                }
                if ctx.node == NodeKind::TypeParameter
                    && !matches!(
                        ctx.type_parameter_owner,
                        TypeParameterOwner::FunctionLike | TypeParameterOwner::Class
                    )
                {
                    return error(1277, span, &["const"]);
                }
            }
            ModifierKind::Override => {
                if flags & FLAG_OVERRIDE != 0 {
                    return error(1030, span, &["override"]);
                } else if flags & FLAG_AMBIENT != 0 {
                    return error(1243, span, &["override", "declare"]);
                } else if flags & FLAG_READONLY != 0 && !modifier.reparsed {
                    return error(1029, span, &["override", "readonly"]);
                } else if flags & FLAG_ACCESSOR != 0 && !modifier.reparsed {
                    return error(1029, span, &["override", "accessor"]);
                } else if flags & FLAG_ASYNC != 0 && !modifier.reparsed {
                    return error(1029, span, &["override", "async"]);
                }
                flags |= FLAG_OVERRIDE;
                last_override = Some(modifier);
            }
            ModifierKind::Public | ModifierKind::Protected | ModifierKind::Private => {
                if flags & FLAG_ACCESSIBILITY != 0 {
                    return error(1028, span, &[]);
                } else if flags & FLAG_OVERRIDE != 0 && !modifier.reparsed {
                    return error(1029, span, &[text, "override"]);
                } else if flags & FLAG_STATIC != 0 && !modifier.reparsed {
                    return error(1029, span, &[text, "static"]);
                } else if flags & FLAG_ACCESSOR != 0 && !modifier.reparsed {
                    return error(1029, span, &[text, "accessor"]);
                } else if flags & FLAG_READONLY != 0 && !modifier.reparsed {
                    return error(1029, span, &[text, "readonly"]);
                } else if flags & FLAG_ASYNC != 0 && !modifier.reparsed {
                    return error(1029, span, &[text, "async"]);
                } else if at_module_level {
                    return error(1044, span, &[text]);
                } else if flags & FLAG_ABSTRACT != 0 {
                    if modifier.kind == ModifierKind::Private {
                        return error(1243, span, &[text, "abstract"]);
                    } else if !modifier.reparsed {
                        return error(1029, span, &[text, "abstract"]);
                    }
                } else if ctx.name_is_private && is_class_member(ctx.node) {
                    return error(18010, span, &[]);
                }
                flags |= accessibility_flag(modifier.kind);
            }
            ModifierKind::Static => {
                if flags & FLAG_STATIC != 0 {
                    return error(1030, span, &["static"]);
                } else if flags & FLAG_READONLY != 0 && !modifier.reparsed {
                    return error(1029, span, &["static", "readonly"]);
                } else if flags & FLAG_ASYNC != 0 && !modifier.reparsed {
                    return error(1029, span, &["static", "async"]);
                } else if flags & FLAG_ACCESSOR != 0 && !modifier.reparsed {
                    return error(1029, span, &["static", "accessor"]);
                } else if at_module_level {
                    return error(1044, span, &["static"]);
                } else if ctx.node == NodeKind::Parameter {
                    return error(1090, span, &["static"]);
                } else if flags & FLAG_ABSTRACT != 0 {
                    return error(1243, span, &["static", "abstract"]);
                } else if flags & FLAG_OVERRIDE != 0 && !modifier.reparsed {
                    return error(1029, span, &["static", "override"]);
                }
                flags |= FLAG_STATIC;
                last_static = Some(modifier);
            }
            ModifierKind::Accessor => {
                if flags & FLAG_ACCESSOR != 0 {
                    return error(1030, span, &["accessor"]);
                } else if flags & FLAG_READONLY != 0 {
                    return error(1243, span, &["accessor", "readonly"]);
                } else if flags & FLAG_AMBIENT != 0 {
                    return error(1243, span, &["accessor", "declare"]);
                } else if ctx.node != NodeKind::PropertyDeclaration {
                    return error(1275, span, &[]);
                }
                flags |= FLAG_ACCESSOR;
            }
            ModifierKind::Readonly => {
                if flags & FLAG_READONLY != 0 {
                    return error(1030, span, &["readonly"]);
                } else if !matches!(ctx.node, NodeKind::PropertyDeclaration | NodeKind::Parameter) {
                    return error(1024, span, &[]);
                } else if flags & FLAG_ACCESSOR != 0 {
                    return error(1243, span, &["readonly", "accessor"]);
                }
                flags |= FLAG_READONLY;
            }
            ModifierKind::Export => {
                if flags & FLAG_EXPORT != 0 {
                    return error(1030, span, &["export"]);
                } else if flags & FLAG_AMBIENT != 0 && !modifier.reparsed {
                    return error(1029, span, &["export", "declare"]);
                } else if flags & FLAG_ABSTRACT != 0 && !modifier.reparsed {
                    return error(1029, span, &["export", "abstract"]);
                } else if flags & FLAG_ASYNC != 0 && !modifier.reparsed {
                    return error(1029, span, &["export", "async"]);
                } else if in_class {
                    return error(1031, span, &["export"]);
                } else if ctx.node == NodeKind::Parameter {
                    return error(1090, span, &["export"]);
                }
                flags |= FLAG_EXPORT;
            }
            ModifierKind::Default => {
                if matches!(ctx.parent, Parent::ModuleBlock { namespace: true }) {
                    return error(1319, span, &[]);
                } else if flags & FLAG_EXPORT == 0 && !modifier.reparsed {
                    return error(1029, span, &["export", "default"]);
                }
                flags |= FLAG_DEFAULT;
            }
            ModifierKind::Declare => {
                if flags & FLAG_AMBIENT != 0 {
                    return error(1030, span, &["declare"]);
                } else if flags & FLAG_ASYNC != 0 {
                    return error(1040, span, &["async"]);
                } else if flags & FLAG_OVERRIDE != 0 {
                    return error(1040, span, &["override"]);
                } else if in_class && ctx.node != NodeKind::PropertyDeclaration {
                    return error(1031, span, &["declare"]);
                } else if ctx.node == NodeKind::Parameter {
                    return error(1090, span, &["declare"]);
                } else if ctx.parent_ambient && matches!(ctx.parent, Parent::ModuleBlock { .. }) {
                    return error(1038, span, &[]);
                } else if ctx.name_is_private && is_class_member(ctx.node) {
                    return error(18019, span, &["declare"]);
                } else if flags & FLAG_ACCESSOR != 0 {
                    return error(1243, span, &["declare", "accessor"]);
                }
                flags |= FLAG_AMBIENT;
            }
            ModifierKind::Abstract => {
                if flags & FLAG_ABSTRACT != 0 {
                    return error(1030, span, &["abstract"]);
                }
                if ctx.node != NodeKind::ClassDeclaration {
                    if !matches!(
                        ctx.node,
                        NodeKind::MethodDeclaration
                            | NodeKind::PropertyDeclaration
                            | NodeKind::GetAccessor
                            | NodeKind::SetAccessor
                    ) {
                        return error(1242, span, &[]);
                    }
                    if !matches!(ctx.parent, Parent::Class { is_declaration: true, is_abstract: true }) {
                        let code = if ctx.node == NodeKind::PropertyDeclaration { 1253 } else { 1244 };
                        return error(code, span, &[]);
                    }
                    if flags & FLAG_STATIC != 0 {
                        return error(1243, span, &["static", "abstract"]);
                    }
                    if flags & FLAG_PRIVATE != 0 {
                        return error(1243, span, &["private", "abstract"]);
                    }
                    if flags & FLAG_ASYNC != 0
                        && let Some(last_async) = last_async
                    {
                        return error(1243, last_async.span, &["async", "abstract"]);
                    }
                    if flags & FLAG_OVERRIDE != 0 && !modifier.reparsed {
                        return error(1029, span, &["abstract", "override"]);
                    }
                    if flags & FLAG_ACCESSOR != 0 && !modifier.reparsed {
                        return error(1029, span, &["abstract", "accessor"]);
                    }
                }
                if ctx.name_is_private {
                    return error(18019, span, &["abstract"]);
                }
                flags |= FLAG_ABSTRACT;
            }
            ModifierKind::Async => {
                if flags & FLAG_ASYNC != 0 {
                    return error(1030, span, &["async"]);
                } else if flags & FLAG_AMBIENT != 0 || ctx.parent_ambient {
                    return error(1040, span, &["async"]);
                } else if ctx.node == NodeKind::Parameter {
                    return error(1090, span, &["async"]);
                }
                if flags & FLAG_ABSTRACT != 0 {
                    return error(1243, span, &["async", "abstract"]);
                }
                flags |= FLAG_ASYNC;
                last_async = Some(modifier);
            }
            ModifierKind::In | ModifierKind::Out => {
                let (flag, text) = if modifier.kind == ModifierKind::In {
                    (FLAG_IN, "in")
                } else {
                    (FLAG_OUT, "out")
                };
                if ctx.node != NodeKind::TypeParameter
                    || !matches!(
                        ctx.type_parameter_owner,
                        TypeParameterOwner::Interface
                            | TypeParameterOwner::Class
                            | TypeParameterOwner::TypeAlias
                    )
                {
                    return error(1274, span, &[text]);
                }
                if flags & flag != 0 {
                    return error(1030, span, &[text]);
                }
                if flag == FLAG_IN && flags & FLAG_OUT != 0 {
                    return error(1029, span, &["in", "out"]);
                }
                flags |= flag;
            }
        }
    }

    if ctx.node == NodeKind::Constructor {
        if flags & FLAG_STATIC != 0 {
            return error(1089, last_static?.span, &["static"]);
        }
        if flags & FLAG_OVERRIDE != 0 {
            return error(1089, last_override?.span, &["override"]);
        }
        if flags & FLAG_ASYNC != 0 {
            return error(1089, last_async?.span, &["async"]);
        }
        return None;
    }
    if flags & FLAG_ASYNC != 0
        && !matches!(ctx.node, NodeKind::MethodDeclaration | NodeKind::FunctionDeclaration)
    {
        return error(1042, last_async?.span, &["async"]);
    }
    None
}

/// tsc's `checkGrammarTopLevelElementsForRequiredDeclareModifier`: in a
/// declaration file every top-level declaration other than an interface, a
/// type alias, an import or an export needs `declare` or `export` — TS1046
/// on the first token of the first one that lacks it, once per file.
pub(crate) fn collect_declaration_file_diagnostics(
    program: &Program<'_>,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    let offending = program.body.iter().find(|statement| {
        matches!(
            statement,
            Statement::FunctionDeclaration(declaration) if !declaration.declare
        ) || matches!(statement, Statement::ClassDeclaration(declaration) if !declaration.declare)
            || matches!(statement, Statement::VariableDeclaration(declaration) if !declaration.declare)
            || matches!(statement, Statement::TSEnumDeclaration(declaration) if !declaration.declare)
            || matches!(statement, Statement::TSModuleDeclaration(declaration) if !declaration.declare)
            || matches!(statement, Statement::TSGlobalDeclaration(declaration) if !declaration.declare)
    });
    if let Some(statement) = offending {
        let start = statement.span().start;
        let end = super::grammar_context::first_token_end(program.source_text, start as usize) as u32;
        out.push(ParsedGrammarDiagnostic {
            kind: Kind::Ts(1046),
            span: text_span_from_oxc_span(Span::new(start, end)),
            name: None,
        });
    }
    report_statements_in_ambient_context(&program.body, program.source_text, out);
    // `checkWithStatement` reports a `with` even where statements are not
    // allowed (TS2410), spanning `with (…)`.
    for statement in &program.body {
        if let Statement::WithStatement(with) = statement {
            let after_object = with.object.span().end as usize;
            if let Some(offset) = program.source_text[after_object..].find(')') {
                out.push(ParsedGrammarDiagnostic {
                    kind: Kind::Ts(2410),
                    span: text_span_from_oxc_span(Span::new(with.span.start, (after_object + offset + 1) as u32)),
                    name: None,
                });
            }
        }
    }
    report_ambient_declarations(&program.body, None, program.source_text, out);
    report_abstract_constructors(&program.body, program.source_text, out);
    // `checkModuleDeclaration`: `declare global` augments nothing in a file
    // that is not a module (TS2669).
    if !super::grammar_context::is_external_module(program) {
        for statement in &program.body {
            if let Statement::TSGlobalDeclaration(global) = statement {
                out.push(ParsedGrammarDiagnostic {
                    kind: Kind::Ts(2669),
                    span: text_span_from_oxc_span(global.global_span),
                    name: None,
                });
            }
        }
    }
    // An ambient `export =` is TS1203 only in a file whose emit format is ESM,
    // which the checker decides.
    for statement in &program.body {
        if let Statement::TSExportAssignment(assignment) = statement {
            out.push(ParsedGrammarDiagnostic {
                kind: Kind::Ts(1203),
                span: text_span_from_oxc_span(assignment.span),
                name: Some(crate::AMBIENT_EXPORT_ASSIGNMENT.to_string()),
            });
        }
    }
}

/// `checkGrammarModifiers`' TS1242 on an `abstract constructor`, which oxc
/// reports only outside a declaration file.
fn report_abstract_constructors(statements: &[Statement<'_>], text: &str, out: &mut Vec<ParsedGrammarDiagnostic>) {
    use oxc_ast::ast::{ClassElement, Declaration, ExportDefaultDeclarationKind, MethodDefinitionKind, MethodDefinitionType};
    for statement in statements {
        let class = match statement {
            Statement::ClassDeclaration(class) => class,
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::ClassDeclaration(class)) => class,
                _ => continue,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => class,
                _ => continue,
            },
            _ => continue,
        };
        for element in &class.body.body {
            let ClassElement::MethodDefinition(method) = element else {
                continue;
            };
            if method.kind != MethodDefinitionKind::Constructor
                || method.r#type != MethodDefinitionType::TSAbstractMethodDefinition
            {
                continue;
            }
            let head = &text[method.span.start as usize..method.key.span().start as usize];
            if let Some(offset) = head.find("abstract") {
                let start = method.span.start + offset as u32;
                out.push(ParsedGrammarDiagnostic {
                    kind: Kind::Ts(1242),
                    span: text_span_from_oxc_span(Span::new(start, start + "abstract".len() as u32)),
                    name: None,
                });
            }
        }
    }
}

/// What tsc's checker reports on a declaration file's declarations, every
/// one of which is ambient: a `declare` modifier on a declaration in a
/// namespace or module body (TS1038, `checkGrammarModifiers`), a namespace
/// declared with the `module` keyword (TS1540, `checkModuleDeclaration`), a
/// function declaration's body (TS1183, `checkBlock`), an export assignment
/// of anything but an entity name (TS2714, `checkExportAssignment`), and an
/// untyped variable (TS7005). `module_block` is whether `statements` are a
/// module body, and whether a namespace's.
fn report_ambient_declarations(
    statements: &[Statement<'_>],
    module_block: Option<bool>,
    text: &str,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind};
    // A namespace body takes no export assignment (TS1063, TS1319), and
    // `checkExportAssignment` stops there.
    let exports_assignable = module_block != Some(true);
    for statement in statements {
        if let Statement::TSExportAssignment(assignment) = statement {
            if exports_assignable {
                report_ambient_export_assignment(&assignment.expression, out);
            }
            continue;
        }
        if let Statement::ExportDefaultDeclaration(export) = statement {
            match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                    report_ambient_function_body(function, out);
                }
                ExportDefaultDeclarationKind::ClassDeclaration(_)
                | ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
                other => {
                    if exports_assignable && let Some(expression) = other.as_expression() {
                        report_ambient_export_assignment(expression, out);
                    }
                }
            }
            continue;
        }
        let declaration = match statement {
            Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
            other => other.as_declaration(),
        };
        let Some(declaration) = declaration else { continue };
        if let Some(namespace) = module_block {
            report_redundant_declare(statement, declaration, namespace, text, out);
        }
        match declaration {
            Declaration::VariableDeclaration(variable) => report_ambient_implicit_any(variable, out),
            Declaration::FunctionDeclaration(function) => report_ambient_function_body(function, out),
            Declaration::TSModuleDeclaration(module) => report_ambient_module(module, text, out),
            Declaration::TSGlobalDeclaration(global) => {
                report_ambient_declarations(&global.body.body, Some(false), text, out);
            }
            _ => {}
        }
    }
}

fn report_ambient_module(
    module: &oxc_ast::ast::TSModuleDeclaration<'_>,
    text: &str,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    use oxc_ast::ast::{TSModuleDeclarationBody, TSModuleDeclarationKind, TSModuleDeclarationName};
    let namespace = match &module.id {
        TSModuleDeclarationName::Identifier(name) => {
            if module.kind == TSModuleDeclarationKind::Module {
                out.push(ParsedGrammarDiagnostic {
                    kind: Kind::Ts(1540),
                    span: text_span_from_oxc_span(name.span),
                    name: None,
                });
            }
            true
        }
        TSModuleDeclarationName::StringLiteral(_) => false,
    };
    match &module.body {
        Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
            report_ambient_declarations(&block.body, Some(namespace), text, out);
        }
        Some(TSModuleDeclarationBody::TSModuleDeclaration(nested)) => report_ambient_module(nested, text, out),
        None => {}
    }
}

/// TS1038 where `checkGrammarModifiers` reaches it on a declaration in a
/// module body: the first modifier error it reports there.
fn report_redundant_declare(
    statement: &Statement<'_>,
    declaration: &oxc_ast::ast::Declaration<'_>,
    namespace: bool,
    text: &str,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    use oxc_ast::ast::Declaration;
    let node = match declaration {
        Declaration::VariableDeclaration(_) => NodeKind::VariableStatement,
        Declaration::FunctionDeclaration(_) => NodeKind::FunctionDeclaration,
        Declaration::ClassDeclaration(_) => NodeKind::ClassDeclaration,
        Declaration::TSTypeAliasDeclaration(_) => NodeKind::TypeAliasDeclaration,
        Declaration::TSInterfaceDeclaration(_) => NodeKind::InterfaceDeclaration,
        Declaration::TSEnumDeclaration(_) => NodeKind::EnumDeclaration,
        Declaration::TSModuleDeclaration(_) | Declaration::TSGlobalDeclaration(_) => NodeKind::ModuleDeclaration,
        Declaration::TSImportEqualsDeclaration(_) => return,
    };
    let Some(modifiers) =
        scan_modifiers(text, statement.span().start, declaration.span().end, node != NodeKind::VariableStatement)
    else {
        return;
    };
    let context = ModifierContext {
        node,
        parent: Parent::ModuleBlock { namespace },
        parent_ambient: true,
        name_is_private: false,
        type_parameter_owner: TypeParameterOwner::Other,
    };
    if let Some(error) = first_modifier_error(&modifiers, &context)
        && error.code == 1038
    {
        out.push(ParsedGrammarDiagnostic {
            kind: Kind::Ts(1038),
            span: text_span_from_oxc_span(error.span),
            name: None,
        });
    }
}

/// `widenTypeForVariableLikeDeclaration`: an ambient variable written with
/// neither a type nor an initializer is an implicit `any` (TS7005, reported
/// under `noImplicitAny`).
fn report_ambient_implicit_any(
    declaration: &oxc_ast::ast::VariableDeclaration<'_>,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    for declarator in &declaration.declarations {
        if declarator.init.is_none()
            && declarator.type_annotation.is_none()
            && let oxc_ast::ast::BindingPattern::BindingIdentifier(identifier) = &declarator.id
        {
            out.push(ParsedGrammarDiagnostic {
                kind: Kind::Ts(7005),
                span: text_span_from_oxc_span(identifier.span),
                name: Some(format!("{}\0any", identifier.name.as_str())),
            });
        }
    }
}

/// TS2714 on an export assignment's expression that is not an entity name.
fn report_ambient_export_assignment(expression: &oxc_ast::ast::Expression<'_>, out: &mut Vec<ParsedGrammarDiagnostic>) {
    if !is_entity_name_expression(expression) {
        out.push(ParsedGrammarDiagnostic {
            kind: Kind::Ts(2714),
            span: text_span_from_oxc_span(expression.span()),
            name: None,
        });
    }
}

/// `ast.IsEntityNameExpression`: an identifier, or a property access on one.
fn is_entity_name_expression(expression: &oxc_ast::ast::Expression<'_>) -> bool {
    use oxc_ast::ast::Expression;
    match expression {
        Expression::Identifier(_) => true,
        Expression::StaticMemberExpression(member) => is_entity_name_expression(&member.object),
        _ => false,
    }
}

/// `checkBlock`'s TS1183 on a function declaration's body, at its `{`. oxc
/// reports it itself on a `declare function`.
fn report_ambient_function_body(function: &oxc_ast::ast::Function<'_>, out: &mut Vec<ParsedGrammarDiagnostic>) {
    if function.declare {
        return;
    }
    if let Some(body) = &function.body {
        out.push(ParsedGrammarDiagnostic {
            kind: Kind::Ts(1183),
            span: text_span_from_oxc_span(Span::new(body.span.start, body.span.start + 1)),
            name: None,
        });
    }
}

/// tsc's `checkGrammarStatementInAmbientContext` over a declaration file,
/// every statement of which is ambient: TS1036 on the first token of the first
/// statement that is not a declaration in each block — the file, a namespace
/// body, or a block, wherever it is nested.
fn report_statements_in_ambient_context(
    statements: &[Statement<'_>],
    text: &str,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    let offending = statements.iter().find(|statement| {
        matches!(
            statement,
            Statement::BlockStatement(_)
                | Statement::IfStatement(_)
                | Statement::DoWhileStatement(_)
                | Statement::WhileStatement(_)
                | Statement::ForStatement(_)
                | Statement::ForInStatement(_)
                | Statement::ForOfStatement(_)
                | Statement::BreakStatement(_)
                | Statement::ContinueStatement(_)
                | Statement::ReturnStatement(_)
                | Statement::WithStatement(_)
                | Statement::SwitchStatement(_)
                | Statement::LabeledStatement(_)
                | Statement::ThrowStatement(_)
                | Statement::TryStatement(_)
                | Statement::ExpressionStatement(_)
                | Statement::EmptyStatement(_)
                | Statement::DebuggerStatement(_)
        )
    });
    if let Some(statement) = offending {
        let start = statement.span().start;
        let end = super::grammar_context::first_token_end(text, start as usize) as u32;
        out.push(ParsedGrammarDiagnostic {
            kind: Kind::Ts(1036),
            span: text_span_from_oxc_span(Span::new(start, end)),
            name: None,
        });
    }
    for statement in statements {
        visit_nested_blocks(statement, text, out);
    }
}

fn visit_nested_blocks(statement: &Statement<'_>, text: &str, out: &mut Vec<ParsedGrammarDiagnostic>) {
    use oxc_ast::ast::{Declaration, TSModuleDeclarationBody};
    let module_block = |declaration: &oxc_ast::ast::TSModuleDeclaration<'_>, out: &mut Vec<ParsedGrammarDiagnostic>| {
        if let Some(TSModuleDeclarationBody::TSModuleBlock(block)) = &declaration.body {
            report_statements_in_ambient_context(&block.body, text, out);
        }
    };
    match statement {
        Statement::BlockStatement(block) => report_statements_in_ambient_context(&block.body, text, out),
        Statement::TSModuleDeclaration(declaration) => module_block(declaration, out),
        Statement::ExportNamedDeclaration(export) => {
            if let Some(Declaration::TSModuleDeclaration(declaration)) = &export.declaration {
                module_block(declaration, out);
            }
        }
        Statement::IfStatement(statement) => {
            visit_nested_blocks(&statement.consequent, text, out);
            if let Some(alternate) = &statement.alternate {
                visit_nested_blocks(alternate, text, out);
            }
        }
        Statement::DoWhileStatement(statement) => visit_nested_blocks(&statement.body, text, out),
        Statement::WhileStatement(statement) => visit_nested_blocks(&statement.body, text, out),
        Statement::ForStatement(statement) => visit_nested_blocks(&statement.body, text, out),
        Statement::ForInStatement(statement) => visit_nested_blocks(&statement.body, text, out),
        Statement::ForOfStatement(statement) => visit_nested_blocks(&statement.body, text, out),
        Statement::LabeledStatement(statement) => visit_nested_blocks(&statement.body, text, out),
        Statement::TryStatement(statement) => {
            report_statements_in_ambient_context(&statement.block.body, text, out);
            if let Some(handler) = &statement.handler {
                report_statements_in_ambient_context(&handler.body.body, text, out);
            }
            if let Some(finalizer) = &statement.finalizer {
                report_statements_in_ambient_context(&finalizer.body, text, out);
            }
        }
        _ => {}
    }
}
