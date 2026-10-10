//! Grammar checks whose answer depends on where a node sits: tsc walks each
//! node's parent chain (`getThisContainer`, `checkGrammarBreakOrContinueStatement`,
//! the binder's strict-mode checks). oxc nodes carry no parent pointers, so
//! this walk keeps the ancestor chain itself and each check reads it the way
//! tsc reads `node.Parent`.
//!
//! oxc and tsc disagree on one shape the ports below account for: a class or
//! object-literal method is a `MethodDefinition`/`ObjectProperty` wrapping a
//! `Function`, where tsc has a single `MethodDeclaration`.

use oxc_ast::AstKind;
use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, Class, Expression, MethodDefinitionKind,
    ModuleExportName, Program, PropertyKind, SimpleAssignmentTarget, Statement,
    VariableDeclarationKind,
};
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, Span};

use super::grammar_modifiers::{self as modifiers, ModifierContext, NodeKind, Parent, TypeParameterOwner};
use super::spans::text_span_from_oxc_span;
use crate::{ParsedGrammarDiagnostic, ParsedGrammarDiagnosticKind as Kind};

mod class_emit;
mod members;
mod reflect_collision;

thread_local! {
    static EXAMINED_MODIFIERS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    static GLOBAL_THIS_STARTS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    static LITERAL_THIS_MEMBERS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    static WITH_BODIES: std::cell::RefCell<Vec<(u32, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The `with` statement bodies the last walk found, as `(start, end)`.
pub(crate) fn take_with_bodies() -> Vec<(u32, u32)> {
    WITH_BODIES.with(|bodies| std::mem::take(&mut *bodies.borrow_mut()))
}

/// Where every modifier the last walk judged starts. The walk is tsc's
/// `checkGrammarModifiers` for those lists, so oxc's own verdict on them
/// (which misses parameters and type parameters, and names the wrong rule
/// for `abstract private`) is dropped.
pub(crate) fn take_examined_modifier_starts() -> Vec<u32> {
    EXAMINED_MODIFIERS.with(|examined| std::mem::take(&mut *examined.borrow_mut()))
}

/// Where every `this` the last walk found the file's top level to own
/// starts, in source order. `tryGetThisTypeAt` answers those with
/// `globalThis` in a script and with `undefined` in a module.
pub(crate) fn take_global_this_starts() -> Vec<u32> {
    GLOBAL_THIS_STARTS.with(|starts| std::mem::take(&mut *starts.borrow_mut()))
}

/// Where every object-literal member the last walk found reading its own
/// `this`, in a literal with no contextual type, starts (see
/// [`crate::ParsedSource::literal_this_members`]), sorted.
pub(crate) fn take_literal_this_members() -> Vec<u32> {
    LITERAL_THIS_MEMBERS.with(|members| std::mem::take(&mut *members.borrow_mut()))
}

pub(crate) fn collect_context_grammar_diagnostics(
    program: &Program<'_>,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    let mut collector = ContextCollector {
        stack: Vec::new(),
        out,
        source_text: program.source_text,
        javascript: program.source_type.is_javascript(),
        external_module: is_external_module(program),
        ambient_depth: 0,
        ambient_reported_statements: Vec::new(),
        with_bodies: Vec::new(),
        ambient_const_enum_names: ambient_const_enum_names(program),
        const_enum_names: unshadowed_const_enum_names(program),
        examined_modifiers: Vec::new(),
        global_this_starts: Vec::new(),
        literal_this_members: Vec::new(),
        infer_declarations: Vec::new(),
        index_signatures_only: false,
    };
    collector.visit_program(program);
    let examined = std::mem::take(&mut collector.examined_modifiers);
    EXAMINED_MODIFIERS.with(|slot| *slot.borrow_mut() = examined);
    // tsc's `checkWithStatement` checks the object but never the body, so no
    // checker grammar error comes from inside one.
    let with_bodies = std::mem::take(&mut collector.with_bodies);
    WITH_BODIES.with(|slot| *slot.borrow_mut() = with_bodies.iter().map(|body| (body.start, body.end)).collect());
    let mut global_this = std::mem::take(&mut collector.global_this_starts);
    global_this.retain(|start| !with_bodies.iter().any(|body| body.start <= *start && *start < body.end));
    GLOBAL_THIS_STARTS.with(|slot| *slot.borrow_mut() = global_this);
    let mut literal_this = std::mem::take(&mut collector.literal_this_members);
    literal_this.retain(|start| !with_bodies.iter().any(|body| body.start <= *start && *start < body.end));
    literal_this.sort_unstable();
    literal_this.dedup();
    LITERAL_THIS_MEMBERS.with(|slot| *slot.borrow_mut() = literal_this);
    if !with_bodies.is_empty() {
        collector.out.retain(|finding| {
            !with_bodies.iter().any(|body| {
                body.start as usize <= finding.span.start && finding.span.end <= body.end as usize
            })
        });
    }
}

/// The index signature grammar of a declaration file the program writes
/// itself, which tsc checks like any other (`checkGrammarIndexSignature`).
pub(crate) fn collect_declaration_file_index_signature_diagnostics(
    program: &Program<'_>,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    let mut collector = ContextCollector {
        stack: Vec::new(),
        out,
        source_text: program.source_text,
        javascript: false,
        external_module: is_external_module(program),
        ambient_depth: 0,
        ambient_reported_statements: Vec::new(),
        with_bodies: Vec::new(),
        ambient_const_enum_names: Vec::new(),
        const_enum_names: Vec::new(),
        examined_modifiers: Vec::new(),
        global_this_starts: Vec::new(),
        literal_this_members: Vec::new(),
        infer_declarations: Vec::new(),
        index_signatures_only: true,
    };
    collector.visit_program(program);
}

/// tsc's `ExternalModuleIndicator` under the default `moduleDetection: auto`:
/// a top-level import or export of any form makes the file a module, and so
/// does an `import.meta` anywhere in it (`getImportMetaIfNecessary`).
pub(crate) fn is_external_module(program: &Program<'_>) -> bool {
    has_top_level_module_syntax(program) || contains_import_meta(program)
}

pub(crate) fn contains_import_meta(program: &Program<'_>) -> bool {
    struct Finder(bool);
    impl<'a> Visit<'a> for Finder {
        fn visit_meta_property(&mut self, meta: &oxc_ast::ast::MetaProperty<'a>) {
            if meta.meta.name == "import" && meta.property.name == "meta" {
                self.0 = true;
            }
        }
    }
    if !program.source_text.contains("meta") {
        return false;
    }
    let mut finder = Finder(false);
    finder.visit_program(program);
    finder.0
}

fn has_top_level_module_syntax(program: &Program<'_>) -> bool {
    program.body.iter().any(|statement| {
        matches!(
            statement,
            Statement::ImportDeclaration(_)
                | Statement::ExportAllDeclaration(_)
                | Statement::ExportDefaultDeclaration(_)
                | Statement::ExportNamedDeclaration(_)
                | Statement::TSExportAssignment(_)
                | Statement::TSNamespaceExportDeclaration(_)
        ) || matches!(
            statement,
            Statement::TSImportEqualsDeclaration(declaration)
                if matches!(
                    declaration.module_reference,
                    oxc_ast::ast::TSModuleReference::ExternalModuleReference(_)
                )
        )
    })
}

struct ContextCollector<'a, 'o> {
    stack: Vec<AstKind<'a>>,
    /// `infer` declarations by the conditional type whose `extends` clause
    /// binds them: (conditional start, name, name span, constraint text).
    infer_declarations: Vec<(u32, &'a str, Span, Option<&'a str>)>,
    out: &'o mut Vec<ParsedGrammarDiagnostic>,
    source_text: &'a str,
    javascript: bool,
    external_module: bool,
    /// How many enclosing nodes carry tsc's `NodeFlagsAmbient`.
    ambient_depth: usize,
    /// Starts of the statements TS1036 was reported on: tsc's
    /// `checkGrammarStatementInAmbientContext` returns true for exactly those,
    /// and a `break`/`continue` then skips its own grammar check.
    ambient_reported_statements: Vec<u32>,
    /// The bodies of the file's `with` statements, which tsc never checks.
    with_bodies: Vec<Span>,
    /// Top-level `const enum` names no other binding in the file reuses, so
    /// an identifier reference to one is the const enum object and nothing
    /// else — what tsc's `isConstEnumObjectType` sees on the expression.
    const_enum_names: Vec<String>,
    /// The `declare const enum`s among `const_enum_names`' candidates.
    ambient_const_enum_names: Vec<String>,
    /// See [`take_examined_modifier_starts`].
    examined_modifiers: Vec<u32>,
    /// See [`take_global_this_starts`].
    global_this_starts: Vec<u32>,
    /// See [`take_literal_this_members`].
    literal_this_members: Vec<u32>,
    /// Only index signatures are checked (a declaration file's walk); the
    /// stack is still kept for the scopes they read.
    index_signatures_only: bool,
}

fn ambient_const_enum_names(program: &Program<'_>) -> Vec<String> {
    program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
            other => other.as_declaration(),
        })
        .filter_map(|declaration| match declaration {
            oxc_ast::ast::Declaration::TSEnumDeclaration(declaration)
                if declaration.r#const && declaration.declare =>
            {
                Some(declaration.id.name.to_string())
            }
            _ => None,
        })
        .collect()
}

/// The top-level `const enum` names whose every binding in the file is such a
/// declaration.
fn unshadowed_const_enum_names(program: &Program<'_>) -> Vec<String> {
    let enum_names: Vec<&str> = program
        .body
        .iter()
        .filter_map(|statement| {
            let declaration = match statement {
                Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                other => other.as_declaration(),
            };
            match declaration {
                Some(oxc_ast::ast::Declaration::TSEnumDeclaration(declaration)) if declaration.r#const => {
                    Some(declaration.id.name.as_str())
                }
                _ => None,
            }
        })
        .collect();
    if enum_names.is_empty() {
        return Vec::new();
    }
    let mut bindings = BindingNames::default();
    bindings.visit_program(program);
    let mut names: Vec<String> = Vec::new();
    for name in &enum_names {
        let bound = bindings.names.iter().filter(|bound| **bound == *name).count();
        let declared = enum_names.iter().filter(|other| **other == *name).count();
        if bound == declared {
            names.push((*name).to_string());
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

#[derive(Default)]
struct BindingNames<'a> {
    names: Vec<&'a str>,
}

impl<'a> Visit<'a> for BindingNames<'a> {
    fn visit_binding_identifier(&mut self, identifier: &oxc_ast::ast::BindingIdentifier<'a>) {
        self.names.push(identifier.name.as_str());
    }
}

fn is_ambient_marker(kind: &AstKind<'_>) -> bool {
    match kind {
        AstKind::Function(function) => function.declare,
        AstKind::Class(class) => class.declare,
        AstKind::VariableDeclaration(declaration) => declaration.declare,
        AstKind::TSModuleDeclaration(declaration) => declaration.declare,
        AstKind::TSEnumDeclaration(declaration) => declaration.declare,
        AstKind::TSGlobalDeclaration(_) => true,
        _ => false,
    }
}

/// Whether an exported declaration exists at run time, where tsc's
/// `checkGrammarModifiers` lets `export` through only on a type alias, an
/// interface or a namespace, and `checkModuleDeclaration` then rejects it on
/// a namespace that is instantiated (const enums count under
/// `verbatimModuleSyntax`, which preserves them).
fn declares_runtime_value(declaration: &oxc_ast::ast::Declaration<'_>) -> bool {
    use oxc_ast::ast::Declaration as D;
    match declaration {
        D::VariableDeclaration(declaration) => !declaration.declare,
        D::FunctionDeclaration(function) => !function.declare,
        D::ClassDeclaration(class) => !class.declare,
        D::TSEnumDeclaration(declaration) => !declaration.declare,
        D::TSImportEqualsDeclaration(_) => true,
        D::TSModuleDeclaration(module) => {
            !module.declare
                && matches!(module.id, oxc_ast::ast::TSModuleDeclarationName::Identifier(_))
                && module_instance_state(module) != ModuleInstanceState::NonInstantiated
        }
        D::TSTypeAliasDeclaration(_) | D::TSInterfaceDeclaration(_) | D::TSGlobalDeclaration(_) => false,
    }
}

/// tsc's `ModuleInstanceState`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ModuleInstanceState {
    NonInstantiated,
    ConstEnumOnly,
    Instantiated,
}

/// tsc's `getModuleInstanceState`: whether a namespace declares anything that
/// exists at run time.
fn module_instance_state(module: &oxc_ast::ast::TSModuleDeclaration<'_>) -> ModuleInstanceState {
    match &module.body {
        None => ModuleInstanceState::Instantiated,
        Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleDeclaration(nested)) => module_instance_state(nested),
        Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block)) => {
            let mut state = ModuleInstanceState::NonInstantiated;
            for statement in &block.body {
                match statement_instance_state(statement, &block.body) {
                    ModuleInstanceState::Instantiated => return ModuleInstanceState::Instantiated,
                    ModuleInstanceState::ConstEnumOnly => state = ModuleInstanceState::ConstEnumOnly,
                    ModuleInstanceState::NonInstantiated => {}
                }
            }
            state
        }
    }
}

fn statement_instance_state(statement: &Statement<'_>, siblings: &[Statement<'_>]) -> ModuleInstanceState {
    match statement {
        Statement::TSInterfaceDeclaration(_) | Statement::TSTypeAliasDeclaration(_) => {
            ModuleInstanceState::NonInstantiated
        }
        Statement::TSEnumDeclaration(declaration) if declaration.r#const => ModuleInstanceState::ConstEnumOnly,
        Statement::ImportDeclaration(_) | Statement::TSImportEqualsDeclaration(_) => {
            ModuleInstanceState::NonInstantiated
        }
        Statement::TSModuleDeclaration(module) => module_instance_state(module),
        Statement::ExportNamedDeclaration(export) => match &export.declaration {
            Some(
                oxc_ast::ast::Declaration::TSInterfaceDeclaration(_)
                | oxc_ast::ast::Declaration::TSTypeAliasDeclaration(_),
            ) => ModuleInstanceState::NonInstantiated,
            Some(oxc_ast::ast::Declaration::TSEnumDeclaration(declaration)) if declaration.r#const => {
                ModuleInstanceState::ConstEnumOnly
            }
            Some(oxc_ast::ast::Declaration::TSModuleDeclaration(module)) => module_instance_state(module),
            Some(_) => ModuleInstanceState::Instantiated,
            // `export { a, b }`: what the names declare
            // (`getModuleInstanceStateForAliasTarget`).
            None if export.source.is_none() => export
                .specifiers
                .iter()
                .map(|specifier| match &specifier.local {
                    ModuleExportName::IdentifierReference(name) => local_instance_state(&name.name, siblings),
                    _ => ModuleInstanceState::Instantiated,
                })
                .max()
                .unwrap_or(ModuleInstanceState::NonInstantiated),
            None => ModuleInstanceState::Instantiated,
        },
        _ => ModuleInstanceState::Instantiated,
    }
}

fn local_instance_state(name: &str, siblings: &[Statement<'_>]) -> ModuleInstanceState {
    let mut found = false;
    let mut state = ModuleInstanceState::NonInstantiated;
    for statement in siblings {
        let declaration_state = match statement {
            Statement::TSInterfaceDeclaration(declaration) if declaration.id.name == name => {
                Some(ModuleInstanceState::NonInstantiated)
            }
            Statement::TSTypeAliasDeclaration(declaration) if declaration.id.name == name => {
                Some(ModuleInstanceState::NonInstantiated)
            }
            Statement::TSEnumDeclaration(declaration) if declaration.id.name == name => Some(if declaration.r#const {
                ModuleInstanceState::ConstEnumOnly
            } else {
                ModuleInstanceState::Instantiated
            }),
            Statement::TSModuleDeclaration(module)
                if matches!(&module.id, oxc_ast::ast::TSModuleDeclarationName::Identifier(id) if id.name == name) =>
            {
                Some(module_instance_state(module))
            }
            _ => None,
        };
        if let Some(declaration_state) = declaration_state {
            found = true;
            state = state.max(declaration_state);
        }
    }
    if found { state } else { ModuleInstanceState::Instantiated }
}

/// The number a constant operand evaluates to, as `evaluate` gives it for a
/// literal (possibly signed or parenthesized).
fn constant_number(expression: &oxc_ast::ast::Expression<'_>) -> Option<f64> {
    match expression {
        oxc_ast::ast::Expression::NumericLiteral(literal) => Some(literal.value),
        oxc_ast::ast::Expression::ParenthesizedExpression(inner) => constant_number(&inner.expression),
        oxc_ast::ast::Expression::UnaryExpression(unary) => {
            let operand = constant_number(&unary.argument)?;
            match unary.operator {
                oxc_syntax::operator::UnaryOperator::UnaryNegation => Some(-operand),
                oxc_syntax::operator::UnaryOperator::UnaryPlus => Some(operand),
                _ => None,
            }
        }
        _ => None,
    }
}

fn is_function_like(kind: &AstKind<'_>) -> bool {
    matches!(kind, AstKind::Function(_) | AstKind::ArrowFunctionExpression(_))
}

fn is_iteration_statement(statement: &Statement<'_>, look_in_labeled_statements: bool) -> bool {
    match statement {
        Statement::ForStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::WhileStatement(_) => true,
        Statement::LabeledStatement(labeled) => {
            look_in_labeled_statements && is_iteration_statement(&labeled.body, true)
        }
        _ => false,
    }
}

fn is_iteration_kind(kind: &AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::ForStatement(_)
            | AstKind::ForInStatement(_)
            | AstKind::ForOfStatement(_)
            | AstKind::DoWhileStatement(_)
            | AstKind::WhileStatement(_)
    )
}

/// A member whose computed key holds `span`: tsc's `getThisContainer` skips
/// the member and its class or object literal for a node in that key.
fn computed_key_contains(kind: &AstKind<'_>, span: Span) -> bool {
    let key_span = match kind {
        AstKind::MethodDefinition(member) if member.computed => member.key.span(),
        AstKind::PropertyDefinition(member) if member.computed => member.key.span(),
        AstKind::AccessorProperty(member) if member.computed => member.key.span(),
        AstKind::ObjectProperty(member) if member.computed => member.key.span(),
        _ => return false,
    };
    key_span.start <= span.start && span.end <= key_span.end
}

fn is_const_type_reference(ty: &oxc_ast::ast::TSType<'_>) -> bool {
    matches!(ty, oxc_ast::ast::TSType::TSTypeReference(reference)
        if reference.type_arguments.is_none()
            && matches!(&reference.type_name, oxc_ast::ast::TSTypeName::IdentifierReference(name) if name.name == "const"))
}

/// What tsc's `getSuperContainer(node, true)` stops at.
#[derive(Clone, Copy)]
enum SuperContainer {
    /// A class method, accessor, constructor, property, or static block.
    ClassMember { is_constructor: bool },
    ObjectMethod,
    Function,
    Arrow,
    Signature,
}

/// The end of the token starting at `start`: an identifier-like run, or one
/// character.
pub(super) fn first_token_end(text: &str, start: usize) -> usize {
    let rest = &text[start..];
    let word = rest
        .char_indices()
        .find(|(_, ch)| !(ch.is_alphanumeric() || *ch == '_' || *ch == '$'))
        .map_or(rest.len(), |(index, _)| index);
    start + word.max(rest.chars().next().map_or(0, char::len_utf8))
}

/// Whether tsc's `nodeCanBeDecorated` accepts a node's decorators with
/// `experimentalDecorators` (`legacy`) and without it (`es`).
#[derive(Clone, Copy)]
struct DecoratorLegality {
    legacy: bool,
    es: bool,
}

impl DecoratorLegality {
    /// `code` under whichever settings reject the decorators.
    fn unless_legal(self, code: u32) -> Option<Kind> {
        match (self.legacy, self.es) {
            (true, true) => None,
            (false, false) => Some(Kind::Ts(code)),
            (false, true) => Some(Kind::TsUnderLegacyDecorators(code)),
            (true, false) => Some(Kind::TsUnderEsDecorators(code)),
        }
    }

    /// `code` under whichever settings accept the decorators.
    fn if_legal(self, code: u32) -> Option<Kind> {
        match (self.legacy, self.es) {
            (true, true) => Some(Kind::Ts(code)),
            (false, false) => None,
            (true, false) => Some(Kind::TsUnderLegacyDecorators(code)),
            (false, true) => Some(Kind::TsUnderEsDecorators(code)),
        }
    }
}

/// What tsc's `getThisContainer(node, false, false)` stops at.
#[derive(Clone, Copy)]
enum ThisContainer<'a> {
    /// A class method, accessor, or constructor.
    ClassMethod(&'a oxc_ast::ast::MethodDefinition<'a>),
    ClassProperty { is_static: bool },
    /// A signature member; `in_interface` when its parent is an interface
    /// body or a class body rather than a type literal.
    Signature { in_interface: bool, is_static: bool },
    /// A plain function declaration or expression.
    Function,
    /// An object-literal method or accessor, which tsc models as a method.
    ObjectMethod,
    Other,
}

impl<'a> ContextCollector<'a, '_> {
    fn push(&mut self, code: u32, span: Span, args: &[&str]) {
        self.out.push(ParsedGrammarDiagnostic {
            kind: Kind::Ts(code),
            span: text_span_from_oxc_span(span),
            name: if args.is_empty() { None } else { Some(args.join("\0")) },
        });
    }

    /// tsc's `checkGrammarTypeArguments` trailing-comma half. Every type
    /// argument list reaches it except an import type's, which tsgo checks
    /// through `checkTypeReferenceOrImport` alone.
    fn check_type_argument_trailing_comma(&mut self, instantiation: &oxc_ast::ast::TSTypeParameterInstantiation<'a>) {
        if matches!(self.stack.last(), Some(AstKind::TSImportType(_))) {
            return;
        }
        let Some(last) = instantiation.params.last() else {
            return;
        };
        let comma = members::skip_trivia(self.source_text, last.span().end as usize);
        if comma < instantiation.span.end as usize && self.source_text.as_bytes()[comma] == b',' {
            self.push(1009, Span::new(comma as u32, comma as u32 + 1), &[]);
        }
    }

    fn in_class(&self) -> bool {
        self.stack.iter().any(|kind| matches!(kind, AstKind::Class(_)))
    }

    fn this_container(&self, node_span: Span) -> ThisContainer<'a> {
        let mut index = self.stack.len();
        let mut child_span = node_span;
        while index > 0 {
            index -= 1;
            let kind = self.stack[index];
            if computed_key_contains(&kind, child_span) {
                // The member, then its class body/object literal, then the class.
                let skip = match kind {
                    AstKind::ObjectProperty(_) => 1,
                    _ => 2,
                };
                index = index.saturating_sub(skip);
                child_span = self.stack[index].span();
                continue;
            }
            let parent = index.checked_sub(1).map(|parent| self.stack[parent]);
            match kind {
                AstKind::Function(_) => {
                    return match parent {
                        Some(AstKind::MethodDefinition(method)) => ThisContainer::ClassMethod(method),
                        Some(AstKind::ObjectProperty(property))
                            if property.method || property.kind != PropertyKind::Init =>
                        {
                            ThisContainer::ObjectMethod
                        }
                        _ => ThisContainer::Function,
                    };
                }
                AstKind::PropertyDefinition(property) => {
                    return ThisContainer::ClassProperty { is_static: property.r#static };
                }
                AstKind::AccessorProperty(property) => {
                    return ThisContainer::ClassProperty { is_static: property.r#static };
                }
                AstKind::TSPropertySignature(_)
                | AstKind::TSMethodSignature(_)
                | AstKind::TSCallSignatureDeclaration(_)
                | AstKind::TSConstructSignatureDeclaration(_)
                | AstKind::TSIndexSignature(_) => {
                    let is_static = matches!(kind, AstKind::TSIndexSignature(signature) if signature.r#static);
                    return ThisContainer::Signature {
                        in_interface: matches!(
                            parent,
                            Some(AstKind::TSInterfaceBody(_) | AstKind::ClassBody(_))
                        ),
                        is_static,
                    };
                }
                AstKind::TSModuleDeclaration(_)
                | AstKind::TSGlobalDeclaration(_)
                | AstKind::StaticBlock(_)
                | AstKind::TSEnumDeclaration(_)
                | AstKind::Program(_) => return ThisContainer::Other,
                _ => {}
            }
            child_span = kind.span();
        }
        ThisContainer::Other
    }

    /// tsc's `getSuperContainer(node, true)`, as a stack index, starting the
    /// walk below `from`.
    fn super_container(&self, from: usize, node_span: Span) -> Option<(usize, SuperContainer)> {
        let mut index = from;
        let mut child_span = node_span;
        while index > 0 {
            index -= 1;
            let kind = self.stack[index];
            if computed_key_contains(&kind, child_span) {
                child_span = kind.span();
                continue;
            }
            let parent = index.checked_sub(1).map(|parent| self.stack[parent]);
            let container = match kind {
                AstKind::Function(_) => match parent {
                    Some(AstKind::MethodDefinition(method)) => Some(SuperContainer::ClassMember {
                        is_constructor: method.kind == MethodDefinitionKind::Constructor,
                    }),
                    Some(AstKind::ObjectProperty(property))
                        if property.method || property.kind != PropertyKind::Init =>
                    {
                        Some(SuperContainer::ObjectMethod)
                    }
                    _ => Some(SuperContainer::Function),
                },
                AstKind::ArrowFunctionExpression(_) => Some(SuperContainer::Arrow),
                AstKind::PropertyDefinition(_)
                | AstKind::AccessorProperty(_)
                | AstKind::StaticBlock(_) => Some(SuperContainer::ClassMember { is_constructor: false }),
                AstKind::TSPropertySignature(_) | AstKind::TSMethodSignature(_) => {
                    Some(SuperContainer::Signature)
                }
                _ => None,
            };
            if let Some(container) = container {
                return Some((index, container));
            }
            child_span = kind.span();
        }
        None
    }

    /// tsc's `checkSuperExpression` placement errors.
    fn check_super(&mut self, span: Span) {
        let is_call = matches!(
            self.stack.last(),
            Some(AstKind::CallExpression(call)) if call.callee.span() == span
        );
        let mut found = self.super_container(self.stack.len(), span);
        if !is_call {
            while let Some((index, SuperContainer::Arrow)) = found {
                found = self.super_container(index, self.stack[index].span());
            }
        }
        let legal = match found {
            Some((_, SuperContainer::ClassMember { is_constructor })) => !is_call || is_constructor,
            Some((_, SuperContainer::ObjectMethod)) => !is_call,
            _ => false,
        };
        if !legal {
            let container_index = found.map_or(0, |(index, _)| index);
            let mut child_span = span;
            let mut in_computed_name = false;
            for kind in self.stack[container_index..].iter().rev() {
                if computed_key_contains(kind, child_span) {
                    in_computed_name = true;
                    break;
                }
                child_span = kind.span();
            }
            // tsc's remaining TS2338 needs a class or object-literal container
            // that is not a member kind, which no TypeScript source produces.
            let code = if in_computed_name {
                2466
            } else if is_call {
                2337
            } else {
                2660
            };
            self.push(code, span, &[]);
            return;
        }
        if let Some((index, SuperContainer::ClassMember { .. })) = found {
            let class = self.stack[..index].iter().rev().find_map(|kind| match kind {
                AstKind::Class(class) => Some(*class),
                _ => None,
            });
            if class.is_some_and(|class| class.super_class.is_none()) {
                self.push(2335, span, &[]);
            }
        }
    }

    /// The placement half of tsc's `checkThisExpression`.
    fn check_this_expression(&mut self, span: Span) {
        let mut index = self.stack.len();
        let mut child_span = span;
        let mut captured_by_arrow = false;
        while index > 0 {
            index -= 1;
            let kind = self.stack[index];
            if computed_key_contains(&kind, child_span) {
                if matches!(kind, AstKind::ObjectProperty(_)) {
                    index = index.saturating_sub(1);
                    child_span = self.stack[index].span();
                    continue;
                }
                self.push(2465, span, &[]);
                return;
            }
            // An object-literal member's `this` is the literal
            // (`getContextualThisParameterType`), never TS2683. Where the
            // literal has no contextual type that is the literal's own type,
            // which the checker gives the members recorded here.
            if let AstKind::Function(function) = kind
                && let Some(literal) = self.object_literal_of_member(index)
            {
                if function.this_param.is_none()
                    && super::jsdoc::this_type_at(function.span.start).is_none()
                    && self.expression_has_no_contextual_type(literal)
                {
                    self.literal_this_members.push(function.span.start);
                }
                return;
            }
            // tsc's `tryGetThisTypeAtEx` finds no `this` for a namespace or enum
            // body, nor for a function declaration without a `this`
            // parameter — TS2683 under `noImplicitThis`.
            match kind {
                AstKind::TSModuleDeclaration(_) | AstKind::TSGlobalDeclaration(_) => {
                    self.push(2331, span, &[]);
                    self.push(2683, span, &[]);
                    return;
                }
                AstKind::TSEnumDeclaration(_) => {
                    self.push(2332, span, &[]);
                    self.push(2683, span, &[]);
                    return;
                }
                AstKind::Function(function)
                    if function.r#type == oxc_ast::ast::FunctionType::FunctionDeclaration
                        && function.this_param.is_none()
                        && super::jsdoc::this_type_at(function.span.start).is_none()
                        && !super::jsdoc::has_opaque_full_signature(function.span.start) =>
                {
                    self.push(2683, span, &[]);
                    return;
                }
                AstKind::Function(function)
                    if function.r#type == oxc_ast::ast::FunctionType::FunctionExpression
                        && function.this_param.is_none()
                        && super::jsdoc::this_type_at(function.span.start).is_none()
                        && self.function_expression_has_no_contextual_this(index) =>
                {
                    self.push(2683, span, &[]);
                    return;
                }
                AstKind::PropertyDefinition(property) => {
                    self.check_this_in_decorated_static_initializer(
                        index,
                        property.r#static,
                        property.value.as_ref(),
                        span,
                    );
                    return;
                }
                AstKind::AccessorProperty(property) => {
                    self.check_this_in_decorated_static_initializer(
                        index,
                        property.r#static,
                        property.value.as_ref(),
                        span,
                    );
                    return;
                }
                AstKind::Function(_)
                | AstKind::StaticBlock(_)
                | AstKind::TSPropertySignature(_)
                | AstKind::TSMethodSignature(_)
                | AstKind::TSCallSignatureDeclaration(_)
                | AstKind::TSConstructSignatureDeclaration(_)
                | AstKind::TSIndexSignature(_) => return,
                // `tryGetThisTypeAt` answers a script file's top level with
                // `globalThis`, and an arrow that captured it is TS7041 under
                // `noImplicitThis`; a module's top-level `this` is `undefined`.
                AstKind::Program(_) => {
                    self.global_this_starts.push(span.start);
                    if captured_by_arrow && !self.external_module {
                        self.push(7041, span, &[]);
                    }
                    return;
                }
                AstKind::ArrowFunctionExpression(_) => captured_by_arrow = true,
                _ => {}
            }
            child_span = kind.span();
        }
    }

    /// tsc's `checkThisInStaticClassFieldInitializerInDecoratedClass`: under
    /// `experimentalDecorators`, `this` in a static property's initializer of
    /// a decorated class — TS2816. `member` is the property's stack index.
    fn check_this_in_decorated_static_initializer(
        &mut self,
        member: usize,
        is_static: bool,
        initializer: Option<&oxc_ast::ast::Expression<'a>>,
        span: Span,
    ) {
        let decorated = matches!(
            member.checked_sub(2).map(|class| self.stack[class]),
            Some(AstKind::Class(class)) if !class.decorators.is_empty()
        );
        let in_initializer = initializer.is_some_and(|initializer| {
            let range = initializer.span();
            range.start <= span.start && span.end <= range.end
        });
        if is_static && decorated && in_initializer {
            self.out.push(ParsedGrammarDiagnostic {
                kind: Kind::TsUnderLegacyDecorators(2816),
                span: text_span_from_oxc_span(span),
                name: None,
            });
        }
    }

    /// tsc's `checkTypeParametersNotReferenced`: a type parameter's default may
    /// name only the parameters declared before it — TS2744 on each reference
    /// to itself or a later one. A nested signature, `infer` capture or mapped
    /// type that redeclares the name hides the outer parameter.
    fn check_type_parameter_defaults(
        &mut self,
        declaration: &oxc_ast::ast::TSTypeParameterDeclaration<'a>,
    ) {
        for (index, parameter) in declaration.params.iter().enumerate() {
            let Some(default) = &parameter.default else {
                continue;
            };
            let mut references = LaterTypeParameterReferences {
                later: declaration.params[index..]
                    .iter()
                    .map(|later| later.name.name.as_str())
                    .collect(),
                shadowed: Vec::new(),
                found: Vec::new(),
            };
            references.visit_ts_type(default);
            for span in references.found {
                self.push(2744, span, &[]);
            }
        }
    }

    /// tsc's `checkGrammarBindingElement`: a rest element with a property name
    /// (`{ ...a: b }`) — TS2566 on the name. oxc keeps the element but drops
    /// the `: b` it parsed as a type annotation, so the name is read from the
    /// source text after the element.
    fn check_rest_binding_property_name(&mut self, rest: &oxc_ast::ast::BindingRestElement<'a>) {
        if !matches!(self.stack.last(), Some(AstKind::ObjectPattern(_))) {
            return;
        }
        let text = self.source_text;
        let after = rest.argument.span().end as usize;
        let Some(colon) = text[after..]
            .find(|ch: char| !ch.is_whitespace())
            .map(|offset| after + offset)
        else {
            return;
        };
        if !text[colon..].starts_with(':') {
            return;
        }
        let Some(start) = text[colon + 1..]
            .find(|ch: char| !ch.is_whitespace())
            .map(|offset| colon + 1 + offset)
        else {
            return;
        };
        let end = match text.as_bytes()[start] {
            b'{' | b'[' => matching_bracket_end(text, start),
            _ => first_token_end(text, start),
        };
        if end > start {
            self.push(2566, Span::new(start as u32, end as u32), &[]);
        }
    }

    /// Whether the function expression at stack `index` sits where tsc's
    /// `getContextualThisParameterType` can find nothing: not an object-literal
    /// member or a member assignment (those give `this` the object), and in a
    /// position with no contextual type at all — an unannotated variable or
    /// class property, an IIFE callee, an expression statement, or a `return`
    /// whose function has no contextual return type — reached through
    /// parentheses, conditionals, logical operators, commas and array
    /// literals. An argument or an annotated target might supply a `this:`
    /// parameter, so it is left alone.
    fn function_expression_has_no_contextual_this(&self, index: usize) -> bool {
        let mut child_span = self.stack[index].span();
        let mut position = index;
        while position > 0 {
            position -= 1;
            match self.stack[position] {
                AstKind::ParenthesizedExpression(_)
                | AstKind::ConditionalExpression(_)
                | AstKind::LogicalExpression(_)
                | AstKind::SequenceExpression(_)
                | AstKind::ArrayExpression(_) => {
                    if let AstKind::ConditionalExpression(conditional) = self.stack[position]
                        && conditional.test.span() == child_span
                    {
                        return false;
                    }
                    child_span = self.stack[position].span();
                }
                AstKind::VariableDeclarator(declarator) => {
                    // An annotation written as a function type without a
                    // `this` parameter supplies no `this` either.
                    let untyped = match declarator.type_annotation.as_ref() {
                        None => {
                            super::jsdoc::declared_type_supplies_this(declarator.span.start) != Some(true)
                        }
                        Some(annotation) => matches!(
                            &annotation.type_annotation,
                            oxc_ast::ast::TSType::TSFunctionType(signature) if signature.this_param.is_none()
                        ),
                    };
                    return untyped
                        && declarator.init.as_ref().is_some_and(|init| init.span() == child_span);
                }
                AstKind::CallExpression(call) => return call.callee.span() == child_span,
                // `exports.f = function () {}` gives `this` no contextual type
                // in a CommonJS module (`getContextualThisParameterType`).
                AstKind::AssignmentExpression(assignment) => {
                    return assignment.right.span() == child_span
                        && super::commonjs::is_commonjs_module()
                        && matches!(
                            assignment.left.as_member_expression().map(|member| member.object()),
                            Some(oxc_ast::ast::Expression::Identifier(object)) if object.name == "exports"
                        );
                }
                AstKind::PropertyDefinition(property) => {
                    return property.value.as_ref().is_some_and(|value| value.span() == child_span)
                        && self.class_property_supplies_no_this(
                            position,
                            property.r#static,
                            property.type_annotation.as_deref(),
                            property.span.start,
                        );
                }
                AstKind::AccessorProperty(property) => {
                    return property.value.as_ref().is_some_and(|value| value.span() == child_span)
                        && self.class_property_supplies_no_this(
                            position,
                            property.r#static,
                            property.type_annotation.as_deref(),
                            property.span.start,
                        );
                }
                AstKind::ExpressionStatement(_) => return self.statement_has_no_contextual_type(position),
                AstKind::ReturnStatement(_) => return self.returns_have_no_contextual_type(position),
                _ => return false,
            }
        }
        false
    }

    /// Whether a class property at stack `member` gives its initializer no
    /// contextual `this` (`getContextualTypeForVariableLikeDeclaration`): an
    /// annotation or JSDoc `@type` is the contextual type, which supplies none
    /// unless it is a function type with a `this` parameter; without one, only
    /// a static property of a class expression has a contextual type.
    fn class_property_supplies_no_this(
        &self,
        member: usize,
        is_static: bool,
        annotation: Option<&oxc_ast::ast::TSTypeAnnotation<'a>>,
        start: u32,
    ) -> bool {
        match annotation {
            Some(annotation) => matches!(
                &annotation.type_annotation,
                oxc_ast::ast::TSType::TSFunctionType(signature) if signature.this_param.is_none()
            ),
            None => match super::jsdoc::declared_type_supplies_this(start) {
                Some(supplies) => !supplies,
                None => !is_static || self.class_of_member_has_no_contextual_type(member),
            },
        }
    }

    /// A class declaration's static property has no contextual type; a class
    /// expression's takes it from the class expression's own
    /// (`getContextualTypeForStaticPropertyDeclaration`).
    fn class_of_member_has_no_contextual_type(&self, member: usize) -> bool {
        match member.checked_sub(2).map(|class| (class, self.stack[class])) {
            Some((_, AstKind::Class(class)))
                if class.r#type == oxc_ast::ast::ClassType::ClassDeclaration =>
            {
                true
            }
            Some((class, AstKind::Class(_))) => self.expression_has_no_contextual_type(class),
            _ => false,
        }
    }

    /// Whether the expression at stack `index` has no contextual type at all
    /// (tsc's `getContextualType` answering nothing), where syntax alone
    /// decides it: an unannotated variable or class property, an IIFE callee,
    /// the left of `&&`, a comma operand before the last, a conditional's
    /// test, an expression statement, or a `return` whose function has no
    /// contextual return type.
    fn expression_has_no_contextual_type(&self, index: usize) -> bool {
        let mut child_span = self.stack[index].span();
        let mut position = index;
        while position > 0 {
            position -= 1;
            match self.stack[position] {
                AstKind::ParenthesizedExpression(_)
                | AstKind::ArrayExpression(_)
                | AstKind::ObjectExpression(_) => {}
                // `as const` passes on its own contextual type
                // (`tryFindWhenConstTypeReference`).
                AstKind::TSAsExpression(assertion) if is_const_type_reference(&assertion.type_annotation) => {}
                AstKind::TSTypeAssertion(assertion) if is_const_type_reference(&assertion.type_annotation) => {}
                // A member's value takes the literal's contextual type's
                // member; a computed key has none.
                AstKind::ObjectProperty(property) => {
                    if property.computed && property.key.span() == child_span {
                        return true;
                    }
                    if property.value.span() != child_span {
                        return false;
                    }
                }
                AstKind::ConditionalExpression(conditional) => {
                    if conditional.test.span() == child_span {
                        return true;
                    }
                }
                AstKind::LogicalExpression(logical) => {
                    let is_and = logical.operator == oxc_syntax::operator::LogicalOperator::And;
                    if logical.left.span() == child_span {
                        if is_and {
                            return true;
                        }
                    } else if !is_and {
                        // The right of `||`/`??` is typed by the left.
                        return false;
                    }
                }
                AstKind::SequenceExpression(sequence) => {
                    if sequence.expressions.last().is_some_and(|last| last.span() != child_span) {
                        return true;
                    }
                }
                AstKind::VariableDeclarator(declarator) => {
                    return declarator.type_annotation.is_none()
                        && matches!(declarator.id, BindingPattern::BindingIdentifier(_))
                        && super::jsdoc::declared_type_at(declarator.span.start).is_none()
                        && declarator.init.as_ref().is_some_and(|init| init.span() == child_span);
                }
                AstKind::PropertyDefinition(property) => {
                    return property.value.as_ref().is_some_and(|value| value.span() == child_span)
                        && property.type_annotation.is_none()
                        && super::jsdoc::declared_type_at(property.span.start).is_none()
                        && (!property.r#static || self.class_of_member_has_no_contextual_type(position));
                }
                AstKind::AccessorProperty(property) => {
                    return property.value.as_ref().is_some_and(|value| value.span() == child_span)
                        && property.type_annotation.is_none()
                        && super::jsdoc::declared_type_at(property.span.start).is_none()
                        && (!property.r#static || self.class_of_member_has_no_contextual_type(position));
                }
                AstKind::CallExpression(call) => return call.callee.span() == child_span,
                AstKind::ExpressionStatement(_) => return self.statement_has_no_contextual_type(position),
                AstKind::ReturnStatement(_) => return self.returns_have_no_contextual_type(position),
                _ => return false,
            }
            child_span = self.stack[position].span();
        }
        false
    }

    /// An expression statement's expression has no contextual type, except an
    /// arrow's expression body, which is the arrow's return value.
    fn statement_has_no_contextual_type(&self, statement: usize) -> bool {
        match (
            statement.checked_sub(1).map(|body| self.stack[body]),
            statement.checked_sub(2).map(|arrow| self.stack[arrow]),
        ) {
            (Some(AstKind::FunctionBody(_)), Some(AstKind::ArrowFunctionExpression(arrow)))
                if arrow.expression =>
            {
                self.function_has_no_contextual_return(statement - 2)
            }
            _ => true,
        }
    }

    /// Whether the `return` at stack `position` has no contextual type.
    fn returns_have_no_contextual_type(&self, position: usize) -> bool {
        self.stack[..position]
            .iter()
            .rposition(|kind| {
                matches!(kind, AstKind::Function(_) | AstKind::ArrowFunctionExpression(_))
            })
            .is_some_and(|function| self.function_has_no_contextual_return(function))
    }

    /// tsc's `getContextualReturnType` finding nothing for the function at
    /// stack `index`: no return type annotation and no contextual signature —
    /// a declaration or a class method has none, a function expression or an
    /// arrow none where it has no contextual type — and, when it is invoked
    /// on the spot, a call without a contextual type.
    fn function_has_no_contextual_return(&self, index: usize) -> bool {
        let written_return = match self.stack[index] {
            AstKind::Function(function) => {
                function.return_type.is_some()
                    || super::jsdoc::return_type_at(function.span.start).is_some()
            }
            AstKind::ArrowFunctionExpression(arrow) => {
                arrow.return_type.is_some() || super::jsdoc::return_type_at(arrow.span.start).is_some()
            }
            _ => return false,
        };
        if written_return {
            return false;
        }
        match (self.stack[index], index.checked_sub(1).map(|parent| self.stack[parent])) {
            (_, Some(AstKind::MethodDefinition(method))) => method.kind == MethodDefinitionKind::Method,
            (_, Some(AstKind::ObjectProperty(_))) => self
                .object_literal_of_member(index)
                .is_some_and(|literal| self.expression_has_no_contextual_type(literal)),
            (AstKind::Function(function), _)
                if function.r#type == oxc_ast::ast::FunctionType::FunctionDeclaration =>
            {
                true
            }
            _ => match self.immediately_invoking_call(index) {
                Some(call) => self.expression_has_no_contextual_type(call),
                None => self.expression_has_no_contextual_type(index),
            },
        }
    }

    /// The stack index of the object literal the function at `index` is a
    /// member of: a method, an accessor, or the value of `key: function`.
    fn object_literal_of_member(&self, index: usize) -> Option<usize> {
        let AstKind::ObjectProperty(property) = self.stack[index.checked_sub(1)?] else {
            return None;
        };
        if property.value.span() != self.stack[index].span() {
            return None;
        }
        let literal = index.checked_sub(2)?;
        matches!(self.stack[literal], AstKind::ObjectExpression(_)).then_some(literal)
    }

    /// The call that invokes the function at stack `index` on the spot, through
    /// parentheses (`GetImmediatelyInvokedFunctionExpression`).
    fn immediately_invoking_call(&self, index: usize) -> Option<usize> {
        let mut child_span = self.stack[index].span();
        let mut position = index;
        while position > 0 {
            position -= 1;
            match self.stack[position] {
                AstKind::ParenthesizedExpression(parenthesized) => child_span = parenthesized.span,
                AstKind::CallExpression(call) if call.callee.span() == child_span => return Some(position),
                _ => return None,
            }
        }
        None
    }

    /// tsc's `checkGrammarStatementInAmbientContext` for a statement directly
    /// in a block: TS1036, once per block.
    fn check_statements_in_ambient_context(&mut self, statements: &[Statement<'_>]) {
        if self.ambient_depth == 0 {
            return;
        }
        let first = statements.iter().find(|statement| {
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
        if let Some(statement) = first {
            let start = statement.span().start;
            let end = first_token_end(self.source_text, start as usize) as u32;
            self.push(1036, Span::new(start, end), &[]);
            self.ambient_reported_statements.push(start);
        }
    }

    /// tsc's `checkGrammarStatementInAmbientContext` for a function's body
    /// block (`checkBlock`): an implementation in an ambient context is TS1183
    /// on its `{`, and the block's own statements report nothing more.
    fn check_implementation_in_ambient_context(&mut self, body: &oxc_ast::ast::FunctionBody<'_>) {
        if self.ambient_depth == 0 {
            return;
        }
        if let Some(AstKind::ArrowFunctionExpression(arrow)) = self.stack.last()
            && arrow.expression
        {
            return;
        }
        self.push(1183, Span::new(body.span.start, body.span.start + 1), &[]);
    }

    /// tsc's `checkMethodDeclaration`: a generator method whose name is the
    /// identifier `constructor` (a string-literal name is not checked).
    fn check_generator_constructor(&mut self, method: &oxc_ast::ast::MethodDefinition<'_>) {
        if method.value.generator
            && !method.computed
            && let oxc_ast::ast::PropertyKey::StaticIdentifier(key) = &method.key
            && key.name == "constructor"
        {
            self.push(1368, key.span, &[]);
        }
    }

    /// tsc's `checkGrammarAccessor`: a class accessor outside an ambient
    /// context needs a body unless it is abstract — `'{' expected` on the
    /// accessor's last character (its `;` when it has one).
    fn check_accessor_body(&mut self, method: &oxc_ast::ast::MethodDefinition<'_>) {
        if !matches!(method.kind, MethodDefinitionKind::Get | MethodDefinitionKind::Set)
            || method.value.body.is_some()
            || method.r#type == oxc_ast::ast::MethodDefinitionType::TSAbstractMethodDefinition
            || self.ambient_depth > 0
        {
            return;
        }
        // tsc checks the accessor only once its modifiers pass
        // (`checkGrammarFunctionLikeDeclaration` first), and these never do on
        // an accessor.
        let modifiers_start = method.decorators.last().map_or(method.span.start, |decorator| decorator.span.end);
        let modifiers = &self.source_text[modifiers_start as usize..method.key.span().start as usize];
        if modifiers
            .split(|c: char| !c.is_ascii_alphabetic())
            .any(|word| matches!(word, "declare" | "readonly" | "async" | "export" | "const" | "accessor"))
        {
            return;
        }
        let end = method.span.end as usize;
        let text = self.source_text;
        let at = if text.as_bytes().get(end.wrapping_sub(1)) == Some(&b';') {
            end - 1
        } else {
            let rest = &text[end..];
            let trimmed = rest.trim_start();
            if trimmed.starts_with(';') { end + (rest.len() - trimmed.len()) } else { end.saturating_sub(1) }
        } as u32;
        self.push(1005, Span::new(at, at + 1), &["{"]);
    }

    /// tsc's shift check in `checkBinaryLikeExpression`, an error only where
    /// the shift is an enum member's initializer: a constant shift of 32 or
    /// more is the shift by its remainder (TS6807).
    /// tsc's `computeEnumMemberValue`: a name whose text is a canonical
    /// number (`1`, `[2]`, `"3"`) is TS2452 at the name.
    fn check_enum_member_numeric_name(&mut self, member: &oxc_ast::ast::TSEnumMember<'_>) {
        use oxc_ast::ast::TSEnumMemberName;
        let (text, span, computed) = match &member.id {
            TSEnumMemberName::String(literal) => (literal.value.as_str(), literal.span, false),
            TSEnumMemberName::ComputedString(literal) => (literal.value.as_str(), literal.span, true),
            TSEnumMemberName::ComputedTemplateString(template) => match template.quasis.first() {
                Some(quasi) if template.quasis.len() == 1 => {
                    (quasi.value.raw.as_str(), template.span, true)
                }
                _ => return,
            },
            _ => return,
        };
        if matches!(text, "Infinity" | "-Infinity" | "NaN") {
            return;
        }
        let Ok(value) = text.parse::<f64>() else {
            return;
        };
        if super::number_text::js_number_to_string(value) != text {
            return;
        }
        let span = if computed {
            let start = self.source_text[..span.start as usize].rfind('[').map_or(span.start, |at| at as u32);
            let end = self.source_text[span.end as usize..]
                .find(']')
                .map_or(span.end, |at| span.end + at as u32 + 1);
            Span::new(start, end)
        } else {
            span
        };
        self.push(2452, span, &[]);
    }

    fn check_enum_member_overshift(&mut self, member: &oxc_ast::ast::TSEnumMember<'_>) {
        let Some(mut expression) = member.initializer.as_ref() else {
            return;
        };
        while let oxc_ast::ast::Expression::ParenthesizedExpression(inner) = expression {
            expression = &inner.expression;
        }
        let oxc_ast::ast::Expression::BinaryExpression(binary) = expression else {
            return;
        };
        let operator = match binary.operator {
            oxc_syntax::operator::BinaryOperator::ShiftLeft => "<<",
            oxc_syntax::operator::BinaryOperator::ShiftRight => ">>",
            oxc_syntax::operator::BinaryOperator::ShiftRightZeroFill => ">>>",
            _ => return,
        };
        let Some(amount) = constant_number(&binary.right) else {
            return;
        };
        if amount.abs() < 32.0 {
            return;
        }
        let left_span = binary.left.span();
        let left = self.source_text[left_span.start as usize..left_span.end as usize].to_string();
        let remainder = amount % 32.0;
        let remainder = super::number_text::js_number_to_string(if remainder == 0.0 { 0.0 } else { remainder });
        self.push(6807, binary.span, &[&left, operator, &remainder]);
    }

    /// tsc's `checkQuestionTokenAgreementBetweenOverloads` for the method
    /// signatures of an interface or type literal: overloads that are neither
    /// all optional nor all required report TS2386 where they differ from the
    /// first, which is canonical with no implementation to defer to.
    fn check_signature_overload_optionality(&mut self, members: &[oxc_ast::ast::TSSignature<'_>]) {
        let mut groups: Vec<(&str, Vec<(bool, Span)>)> = Vec::new();
        for member in members {
            let oxc_ast::ast::TSSignature::TSMethodSignature(method) = member else {
                continue;
            };
            if method.kind != oxc_ast::ast::TSMethodSignatureKind::Method {
                continue;
            }
            let name = match &method.key {
                oxc_ast::ast::PropertyKey::StaticIdentifier(identifier) => identifier.name.as_str(),
                oxc_ast::ast::PropertyKey::StringLiteral(literal) => literal.value.as_str(),
                _ => continue,
            };
            let overload = (method.optional, method.key.span());
            match groups.iter_mut().find(|(existing, _)| *existing == name) {
                Some((_, overloads)) => overloads.push(overload),
                None => groups.push((name, vec![overload])),
            }
        }
        for (_, overloads) in groups {
            let some_optional = overloads.iter().any(|(optional, _)| *optional);
            let all_optional = overloads.iter().all(|(optional, _)| *optional);
            if some_optional == all_optional {
                continue;
            }
            let canonical = overloads[0].0;
            for (optional, span) in overloads {
                if optional != canonical {
                    self.push(2386, span, &[]);
                }
            }
        }
    }

    /// The constructs `erasableSyntaxOnly` forbids (TS1294), recorded always
    /// and reported only under the option: a parameter property, a non-ambient
    /// enum, an instantiated non-ambient namespace, a non-ambient `import =`
    /// or `export =`, and a `<T>x` assertion.
    fn push_erasable(&mut self, span: Span) {
        self.push(1294, span, &[]);
    }

    /// A `declare` written on a declaration already inside an ambient
    /// namespace body — TS1038.
    fn check_redundant_declare(&mut self, statements: &[Statement<'_>]) {
        if self.ambient_depth == 0 {
            return;
        }
        for statement in statements {
            let declaration = match statement {
                Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                other => other.as_declaration(),
            };
            let declared = match declaration {
                Some(oxc_ast::ast::Declaration::VariableDeclaration(d)) => d.declare,
                Some(oxc_ast::ast::Declaration::FunctionDeclaration(d)) => d.declare,
                Some(oxc_ast::ast::Declaration::ClassDeclaration(d)) => d.declare,
                Some(oxc_ast::ast::Declaration::TSTypeAliasDeclaration(d)) => d.declare,
                Some(oxc_ast::ast::Declaration::TSInterfaceDeclaration(d)) => d.declare,
                Some(oxc_ast::ast::Declaration::TSEnumDeclaration(d)) => d.declare,
                Some(oxc_ast::ast::Declaration::TSModuleDeclaration(d)) => d.declare,
                _ => false,
            };
            if !declared {
                continue;
            }
            let from = statement.span().start as usize;
            if let Some(offset) = self.source_text[from..statement.span().end as usize].find("declare") {
                let start = (from + offset) as u32;
                self.push(1038, Span::new(start, start + 7), &[]);
            }
        }
    }

    /// The syntactic half of tsc's `checkGrammarIndexSignatureParameters`:
    /// a literal or type-parameter key is TS1337, a keyword key that is not
    /// `string`/`number`/`symbol` is TS1268. A key naming an alias is left to
    /// types this pass does not have.
    fn check_index_signature(&mut self, signature: &oxc_ast::ast::TSIndexSignature<'_>) {
        let [parameter] = signature.parameters.as_slice() else {
            return;
        };
        // The parser's stand-in for a missing key type, after a parameter
        // error tsc stops at.
        if parameter.type_annotation.span.is_empty() {
            return;
        }
        let key = &parameter.type_annotation.type_annotation;
        let name_span = Span::new(parameter.span.start, parameter.span.start + parameter.name.len() as u32);
        let members: Vec<&oxc_ast::ast::TSType<'_>> = match key {
            oxc_ast::ast::TSType::TSUnionType(union) => union.types.iter().collect(),
            other => vec![other],
        };
        let mut invalid = false;
        for member in &members {
            match member {
                oxc_ast::ast::TSType::TSLiteralType(_) => {
                    self.push(1337, name_span, &[]);
                    return;
                }
                oxc_ast::ast::TSType::TSTypeReference(reference) => {
                    if let oxc_ast::ast::TSTypeName::IdentifierReference(name) = &reference.type_name
                        && self.is_type_parameter_in_scope(&name.name)
                    {
                        self.push(1337, name_span, &[]);
                        return;
                    }
                    if members.len() == 1
                        && reference.type_arguments.is_none()
                        && let oxc_ast::ast::TSTypeName::IdentifierReference(name) = &reference.type_name
                        && self.type_names_resolve_at_top_level()
                    {
                        self.out.push(ParsedGrammarDiagnostic {
                            kind: Kind::IndexSignatureKeyReference,
                            span: text_span_from_oxc_span(name_span),
                            name: Some(format!("{}\0{}\0{}", name.name, reference.span.start, reference.span.end)),
                        });
                    }
                    return;
                }
                oxc_ast::ast::TSType::TSStringKeyword(_)
                | oxc_ast::ast::TSType::TSNumberKeyword(_)
                | oxc_ast::ast::TSType::TSSymbolKeyword(_)
                | oxc_ast::ast::TSType::TSTemplateLiteralType(_) => {}
                oxc_ast::ast::TSType::TSAnyKeyword(_)
                | oxc_ast::ast::TSType::TSUnknownKeyword(_)
                | oxc_ast::ast::TSType::TSBooleanKeyword(_)
                | oxc_ast::ast::TSType::TSBigIntKeyword(_)
                | oxc_ast::ast::TSType::TSObjectKeyword(_)
                | oxc_ast::ast::TSType::TSNeverKeyword(_)
                | oxc_ast::ast::TSType::TSVoidKeyword(_)
                | oxc_ast::ast::TSType::TSNullKeyword(_)
                | oxc_ast::ast::TSType::TSUndefinedKeyword(_) => invalid = true,
                // `isGenericType`: a type parameter constituent makes the
                // intersection generic, whatever it reduces to.
                oxc_ast::ast::TSType::TSIntersectionType(intersection) => {
                    if intersection.types.iter().any(|member| self.type_parameter_key_name(member).is_some()) {
                        self.push(1337, name_span, &[]);
                    }
                    return;
                }
                _ => return,
            }
        }
        if invalid {
            self.push(1268, name_span, &[]);
        }
    }

    fn is_type_parameter_in_scope(&self, name: &str) -> bool {
        self.type_parameter_in_scope(name).is_some()
    }

    /// Whether a type name written here resolves in the file's top-level
    /// scope: nothing encloses it that declares types of its own (a function,
    /// block, namespace or `declare global`) or type parameters
    /// `type_parameter_in_scope` does not see.
    fn type_names_resolve_at_top_level(&self) -> bool {
        !self.stack.iter().any(|kind| {
            matches!(
                kind,
                AstKind::Function(_)
                    | AstKind::ArrowFunctionExpression(_)
                    | AstKind::FunctionBody(_)
                    | AstKind::BlockStatement(_)
                    | AstKind::StaticBlock(_)
                    | AstKind::TSModuleDeclaration(_)
                    | AstKind::TSGlobalDeclaration(_)
                    | AstKind::TSMappedType(_)
                    | AstKind::TSFunctionType(_)
                    | AstKind::TSConstructorType(_)
                    | AstKind::TSCallSignatureDeclaration(_)
                    | AstKind::TSConstructSignatureDeclaration(_)
                    | AstKind::TSConditionalType(_)
            )
        })
    }

    /// The innermost in-scope declaration of the type parameter `name`.
    fn type_parameter_in_scope(&self, name: &str) -> Option<&oxc_ast::ast::TSTypeParameter<'a>> {
        self.stack.iter().rev().find_map(|kind| {
            let parameters = match kind {
                AstKind::TSInterfaceDeclaration(d) => d.type_parameters.as_deref(),
                AstKind::TSTypeAliasDeclaration(d) => d.type_parameters.as_deref(),
                AstKind::Class(d) => d.type_parameters.as_deref(),
                AstKind::Function(d) => d.type_parameters.as_deref(),
                AstKind::ArrowFunctionExpression(d) => d.type_parameters.as_deref(),
                AstKind::TSMethodSignature(d) => d.type_parameters.as_deref(),
                _ => None,
            };
            parameters?.params.iter().find(|p| p.name.name == name)
        })
    }

    /// The in-scope type parameter a key type names.
    fn type_parameter_key_name(&self, ty: &oxc_ast::ast::TSType<'_>) -> Option<String> {
        let oxc_ast::ast::TSType::TSTypeReference(reference) = ty else {
            return None;
        };
        let oxc_ast::ast::TSTypeName::IdentifierReference(name) = &reference.type_name else {
            return None;
        };
        (reference.type_arguments.is_none() && self.is_type_parameter_in_scope(&name.name))
            .then(|| name.name.to_string())
    }

    /// `getIntersectionType` reduces `T & P` to `T` when `T`'s constraint is a
    /// strict subtype of the primitive `P`; syntactically, when the constraint
    /// is written as that same keyword.
    fn reduced_type_parameter_intersection(
        &self,
        intersection: &oxc_ast::ast::TSIntersectionType<'_>,
    ) -> Option<String> {
        let [first, second] = intersection.types.as_slice() else {
            return None;
        };
        let (parameter, primitive) = match self.type_parameter_key_name(first) {
            Some(parameter) => (parameter, second),
            None => (self.type_parameter_key_name(second)?, first),
        };
        let keyword = keyword_text(primitive)
            .filter(|keyword| matches!(*keyword, "string" | "number" | "bigint" | "boolean" | "symbol" | "object"))?;
        let constraint = self.type_parameter_in_scope(&parameter)?.constraint.as_ref()?;
        (keyword_text(constraint) == Some(keyword)).then_some(parameter)
    }

    /// tsc's `checkTypeForDuplicateIndexSignatures` for keys written as a
    /// keyword, a template literal type or a type parameter, compared by their
    /// text.
    fn check_duplicate_index_signatures<'s>(
        &mut self,
        signatures: impl Iterator<Item = &'s oxc_ast::ast::TSIndexSignature<'s>>,
    ) {
        let mut keys: Vec<(bool, String, Span)> = Vec::new();
        for signature in signatures {
            let [parameter] = signature.parameters.as_slice() else {
                continue;
            };
            let key = &parameter.type_annotation.type_annotation;
            let members: Vec<&oxc_ast::ast::TSType<'_>> = match key {
                oxc_ast::ast::TSType::TSUnionType(union) => union.types.iter().collect(),
                other => vec![other],
            };
            for member in members {
                let text = match member {
                    oxc_ast::ast::TSType::TSStringKeyword(_) => "string".to_string(),
                    oxc_ast::ast::TSType::TSNumberKeyword(_) => "number".to_string(),
                    oxc_ast::ast::TSType::TSSymbolKeyword(_) => "symbol".to_string(),
                    oxc_ast::ast::TSType::TSTemplateLiteralType(template) => {
                        let span = template.span;
                        self.source_text[span.start as usize..span.end as usize].to_string()
                    }
                    oxc_ast::ast::TSType::TSTypeReference(_) => match self.type_parameter_key_name(member) {
                        Some(name) => name,
                        None => continue,
                    },
                    oxc_ast::ast::TSType::TSIntersectionType(intersection) => {
                        match self.reduced_type_parameter_intersection(intersection) {
                            Some(name) => name,
                            None => continue,
                        }
                    }
                    _ => continue,
                };
                keys.push((signature.r#static, text, signature.span));
            }
        }
        for (is_static, key, span) in &keys {
            if keys.iter().filter(|(s, k, _)| s == is_static && k == key).count() > 1 {
                self.push(2374, *span, &[key]);
            }
        }
    }

    /// The binder's merge conflict for an enum: a function, class, variable,
    /// interface, or type alias of the same name in the same scope — TS2567
    /// at every declaration involved. Namespaces merge with enums; an import
    /// conflict is its own error.
    fn check_enum_merges(&mut self, statements: &[Statement<'_>]) {
        let mut declarations: Vec<(&str, bool, bool, Span)> = Vec::new();
        for statement in statements {
            let (declaration, exported) = match statement {
                Statement::ExportNamedDeclaration(export) => (export.declaration.as_ref(), true),
                other => (other.as_declaration(), false),
            };
            let Some(declaration) = declaration else {
                continue;
            };
            use oxc_ast::ast::Declaration as D;
            match declaration {
                D::TSEnumDeclaration(d) => declarations.push((d.id.name.as_str(), true, exported, d.id.span)),
                D::FunctionDeclaration(d) => {
                    if let Some(id) = &d.id {
                        declarations.push((id.name.as_str(), false, exported, id.span));
                    }
                }
                D::ClassDeclaration(d) => {
                    if let Some(id) = &d.id {
                        declarations.push((id.name.as_str(), false, exported, id.span));
                    }
                }
                D::TSInterfaceDeclaration(d) => declarations.push((d.id.name.as_str(), false, exported, d.id.span)),
                D::TSTypeAliasDeclaration(d) => declarations.push((d.id.name.as_str(), false, exported, d.id.span)),
                D::VariableDeclaration(d) => {
                    for declarator in &d.declarations {
                        if let BindingPattern::BindingIdentifier(id) = &declarator.id {
                            declarations.push((id.name.as_str(), false, exported, id.span));
                        }
                    }
                }
                _ => {}
            }
        }
        let enum_names: Vec<(&str, bool)> = declarations
            .iter()
            .filter(|(_, is_enum, _, _)| *is_enum)
            .map(|(name, _, exported, _)| (*name, *exported))
            .collect();
        for (name, _, exported, span) in &declarations {
            let conflicts = enum_names.iter().any(|(n, e)| n == name && e == exported)
                && declarations
                    .iter()
                    .any(|(n, is_enum, e, _)| n == name && !is_enum && e == exported);
            if conflicts {
                self.push(2567, *span, &[]);
            }
        }
    }

    /// tsc's `checkConstEnumAccess`: the const enum object may only be the
    /// object of a property or element access, the right side of an import
    /// or export assignment, a `typeof` query, or an export specifier —
    /// TS2475 anywhere else. A type-position name is not an expression and is
    /// never checked.
    fn check_const_enum_reference(&mut self, identifier: &oxc_ast::ast::IdentifierReference<'_>) {
        if !self.const_enum_names.iter().any(|name| name == identifier.name.as_str()) {
            return;
        }
        let allowed = match self.stack.last() {
            Some(AstKind::StaticMemberExpression(member)) => member.object.span() == identifier.span,
            Some(AstKind::ComputedMemberExpression(member)) => member.object.span() == identifier.span,
            Some(
                AstKind::ExportDefaultDeclaration(_)
                | AstKind::TSExportAssignment(_)
                | AstKind::TSImportEqualsDeclaration(_)
                | AstKind::ExportSpecifier(_)
                | AstKind::TSTypeQuery(_)
                | AstKind::TSTypeReference(_)
                | AstKind::TSQualifiedName(_)
                | AstKind::TSClassImplements(_),
            ) => true,
            _ => false,
        };
        if !allowed {
            self.push(2475, identifier.span, &[]);
        }
        // The rest of `checkConstEnumAccess`: an ambient const enum has no
        // value a single-file transpiler could inline (TS2748, decided by the
        // checker from the options). A type position never reaches it.
        let type_position = matches!(
            self.stack.last(),
            Some(
                AstKind::TSTypeQuery(_)
                    | AstKind::TSTypeReference(_)
                    | AstKind::TSQualifiedName(_)
                    | AstKind::TSClassImplements(_)
                    | AstKind::TSImportEqualsDeclaration(_)
                    | AstKind::ExportSpecifier(_)
            )
        );
        if !type_position
            && self.ambient_depth == 0
            && self.ambient_const_enum_names.iter().any(|name| name == identifier.name.as_str())
        {
            self.push(2748, identifier.span, &[if allowed { "access" } else { "" }]);
        }
    }

    /// tsc's `checkElementAccessExpression` on a const enum object: the index
    /// must be a string literal or an untemplated template — TS2476 at it.
    fn check_const_enum_element_access(&mut self, member: &oxc_ast::ast::ComputedMemberExpression<'_>) {
        let oxc_ast::ast::Expression::Identifier(object) = &member.object else {
            return;
        };
        if !self.const_enum_names.iter().any(|name| name == object.name.as_str()) {
            return;
        }
        let is_string_literal_like = match &member.expression {
            oxc_ast::ast::Expression::StringLiteral(_) => true,
            oxc_ast::ast::Expression::TemplateLiteral(template) => template.expressions.is_empty(),
            _ => false,
        };
        if !is_string_literal_like {
            self.push(2476, member.expression.span(), &[]);
        }
    }

    fn check_merged_declarations(&mut self, statements: &[Statement<'_>]) {
        super::grammar_merges::check_merged_declarations(
            statements,
            self.ambient_depth > 0,
            self.source_text,
            self.out,
        );
    }

    /// tsc's `checkModuleDeclaration`: an ambient module at the top of a
    /// script file may not use a relative name — TS2436. In a module file the
    /// same declaration is an augmentation, which the checker resolves.
    fn check_relative_ambient_module_name(&mut self, module: &oxc_ast::ast::TSModuleDeclaration<'_>) {
        let oxc_ast::ast::TSModuleDeclarationName::StringLiteral(name) = &module.id else {
            return;
        };
        if self.external_module || !matches!(self.stack.last(), Some(AstKind::Program(_))) {
            return;
        }
        if is_external_module_name_relative(name.value.as_str()) {
            self.push(2436, name.span, &[]);
        }
    }

    /// tsc's `checkVarDeclaredNamesNotShadowed`: a `var` hoists past the
    /// block-scoped declaration of the same name in any enclosing scope short
    /// of its own function, namespace, or file, which at run time is a
    /// syntax error — TS2481 at the declarator. A block-scoped declaration in
    /// the hoisting scope itself is the binder's redeclaration error instead.
    fn check_var_declared_name_not_shadowed(&mut self, declarator: &oxc_ast::ast::VariableDeclarator<'_>) {
        if declarator.kind != VariableDeclarationKind::Var {
            return;
        }
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            return;
        };
        let name = id.name.as_str();
        for kind in self.stack.iter().rev() {
            let shadowed = match kind {
                AstKind::BlockStatement(block) => statements_declare_block_scoped(&block.body, name),
                AstKind::SwitchStatement(statement) => statement
                    .cases
                    .iter()
                    .any(|case| statements_declare_block_scoped(&case.consequent, name)),
                AstKind::ForStatement(statement) => matches!(
                    &statement.init,
                    Some(oxc_ast::ast::ForStatementInit::VariableDeclaration(declaration))
                        if variable_declaration_declares_block_scoped(declaration, name)
                ),
                AstKind::ForInStatement(statement) => matches!(
                    &statement.left,
                    oxc_ast::ast::ForStatementLeft::VariableDeclaration(declaration)
                        if variable_declaration_declares_block_scoped(declaration, name)
                ),
                AstKind::ForOfStatement(statement) => matches!(
                    &statement.left,
                    oxc_ast::ast::ForStatementLeft::VariableDeclaration(declaration)
                        if variable_declaration_declares_block_scoped(declaration, name)
                ),
                AstKind::FunctionBody(_)
                | AstKind::StaticBlock(_)
                | AstKind::TSModuleBlock(_)
                | AstKind::Program(_) => return,
                _ => false,
            };
            if shadowed {
                self.push(2481, declarator.span, &[name, name]);
                return;
            }
        }
    }

    /// tsc's `checkTypeParameterListsIdentical` over the class and interface
    /// declarations of one name in one scope — TS2428 on each. Constraints are
    /// compared only where both are a keyword, which is identity without types.
    fn check_type_parameter_lists_identical(&mut self, statements: &[Statement<'_>]) {
        type Params<'s> = Option<&'s oxc_ast::ast::TSTypeParameterDeclaration<'s>>;
        // A namespace's exported members merge across all of its blocks; the
        // qualified name keys them apart from same-named members elsewhere.
        fn collect<'s>(
            statements: &'s [Statement<'s>],
            prefix: &str,
            exported_only: bool,
            out: &mut Vec<(String, &'s str, Span, Params<'s>)>,
        ) {
            use oxc_ast::ast::Declaration as D;
            for statement in statements {
                let (declaration, exported) = match statement {
                    Statement::ExportNamedDeclaration(export) => (export.declaration.as_ref(), true),
                    other => (other.as_declaration(), false),
                };
                match declaration {
                    Some(D::TSInterfaceDeclaration(d)) if exported || !exported_only => {
                        let name = d.id.name.as_str();
                        out.push((format!("{prefix}{name}"), name, d.id.span, d.type_parameters.as_deref()));
                    }
                    Some(D::ClassDeclaration(d)) if exported || !exported_only => {
                        if let Some(id) = &d.id {
                            let name = id.name.as_str();
                            out.push((format!("{prefix}{name}"), name, id.span, d.type_parameters.as_deref()));
                        }
                    }
                    Some(D::TSModuleDeclaration(module)) if exported || !exported_only => {
                        if let oxc_ast::ast::TSModuleDeclarationName::Identifier(id) = &module.id
                            && let Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block)) =
                                module.body.as_ref()
                        {
                            collect(&block.body, &format!("{prefix}{}.", id.name), true, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut declarations: Vec<(String, &str, Span, Params<'_>)> = Vec::new();
        collect(statements, "", false, &mut declarations);
        // An alias can spell one type two ways, so a reference to one is
        // compared by nothing but identity of text with itself.
        fn alias_names<'s>(statements: &'s [Statement<'s>], out: &mut Vec<&'s str>) {
            use oxc_ast::ast::Declaration as D;
            for statement in statements {
                let declaration = match statement {
                    Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                    other => other.as_declaration(),
                };
                match declaration {
                    Some(D::TSTypeAliasDeclaration(alias)) => out.push(alias.id.name.as_str()),
                    Some(D::TSModuleDeclaration(module)) => {
                        if let Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block)) = module.body.as_ref() {
                            alias_names(&block.body, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut aliases = Vec::new();
        alias_names(statements, &mut aliases);
        let source_text = self.source_text;
        let names_alias = |ty: &oxc_ast::ast::TSType<'_>| match ty {
            oxc_ast::ast::TSType::TSTypeReference(reference) => match &reference.type_name {
                oxc_ast::ast::TSTypeName::IdentifierReference(name) => aliases.contains(&name.name.as_str()),
                _ => true,
            },
            _ => false,
        };
        let text_of = |ty: &oxc_ast::ast::TSType<'_>| {
            let span = ty.span();
            source_text[span.start as usize..span.end as usize].split_whitespace().collect::<String>()
        };
        let types_differ = |left: &oxc_ast::ast::TSType<'_>, right: &oxc_ast::ast::TSType<'_>| {
            if keywords_differ(left, right) {
                return true;
            }
            let is_reference = |ty: &oxc_ast::ast::TSType<'_>| matches!(ty, oxc_ast::ast::TSType::TSTypeReference(_));
            let comparable = |ty: &oxc_ast::ast::TSType<'_>| is_reference(ty) || keyword_text(ty).is_some();
            (is_reference(left) || is_reference(right))
                && comparable(left)
                && comparable(right)
                && !names_alias(left)
                && !names_alias(right)
                && text_of(left) != text_of(right)
        };
        let mut checked: Vec<&str> = Vec::new();
        for (key, name, _, _) in &declarations {
            if checked.contains(&key.as_str()) {
                continue;
            }
            checked.push(key.as_str());
            let group: Vec<&(String, &str, Span, Params<'_>)> =
                declarations.iter().filter(|(other, _, _, _)| other == key).collect();
            if group.len() < 2 {
                continue;
            }
            // The merged type's parameters: every name in order of first
            // appearance, the first declaration of each supplying its default
            // and constraint.
            type Merged<'t> = (&'t str, Option<&'t oxc_ast::ast::TSType<'t>>, Option<&'t oxc_ast::ast::TSType<'t>>);
            let mut merged: Vec<Merged<'_>> = Vec::new();
            for (_, _, _, params) in &group {
                for param in params.iter().flat_map(|params| params.params.iter()) {
                    let param_name = param.name.name.as_str();
                    match merged.iter_mut().find(|(merged_name, _, _)| *merged_name == param_name) {
                        Some(entry) => {
                            if entry.1.is_none() {
                                entry.1 = param.default.as_ref();
                            }
                            if entry.2.is_none() {
                                entry.2 = param.constraint.as_ref();
                            }
                        }
                        None => merged.push((param_name, param.default.as_ref(), param.constraint.as_ref())),
                    }
                }
            }
            let min = merged.iter().take_while(|(_, default, _)| default.is_none()).count();
            let identical = group.iter().all(|(_, _, _, params)| {
                let params: Vec<&oxc_ast::ast::TSTypeParameter<'_>> =
                    params.iter().flat_map(|params| params.params.iter()).collect();
                params.len() >= min
                    && params.len() <= merged.len()
                    && params.iter().zip(&merged).all(|(param, (merged_name, default, constraint))| {
                        let same = |own: Option<&oxc_ast::ast::TSType<'_>>, merged: &Option<&oxc_ast::ast::TSType<'_>>| {
                            match (own, merged) {
                                (Some(own), Some(merged)) => !types_differ(own, merged),
                                _ => true,
                            }
                        };
                        param.name.name == *merged_name
                            && same(param.constraint.as_ref(), constraint)
                            && same(param.default.as_ref(), default)
                    })
            });
            if !identical {
                for (_, _, span, _) in &group {
                    self.push(2428, *span, &[name]);
                }
            }
        }
    }

    /// tsc's `checkExternalModuleExports`: `export =` beside another value
    /// export — TS2309 on the assignment. An export whose meaning surge
    /// cannot see (a re-export, an imported name) is taken to be a type.
    fn check_export_assignment_conflicts(&mut self, program: &Program<'_>) {
        self.check_export_assignment_conflicts_in(&program.body);
    }

    /// The same for each `declare module "m" { … }` body, a module of its own.
    fn check_ambient_module_export_assignments(&mut self, program: &Program<'_>) {
        for statement in &program.body {
            if let Statement::TSModuleDeclaration(module) = statement
                && matches!(module.id, oxc_ast::ast::TSModuleDeclarationName::StringLiteral(_))
                && let Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block)) = &module.body
            {
                self.check_export_assignment_conflicts_in(&block.body);
            }
        }
    }

    fn check_export_assignment_conflicts_in(&mut self, body: &[Statement<'_>]) {
        let Some(assignment) = body.iter().find_map(|statement| match statement {
            Statement::TSExportAssignment(assignment) => Some(assignment),
            _ => None,
        }) else {
            return;
        };
        let local_value = |name: &str| {
            body.iter().any(|statement| {
                statement_declares_value(statement, name)
            })
        };
        let exports_value = body.iter().any(|statement| match statement {
            Statement::ExportDefaultDeclaration(export) => !matches!(
                export.declaration,
                oxc_ast::ast::ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
            ),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                // `export import a = x.c` is an exported alias whatever it names.
                Some(oxc_ast::ast::Declaration::TSImportEqualsDeclaration(_)) => true,
                Some(declaration) => declaration_is_value(declaration),
                None => {
                    export.source.is_none()
                        && export.export_kind.is_value()
                        && export.specifiers.iter().any(|specifier| {
                            specifier.export_kind.is_value() && local_value(&specifier.local.name())
                        })
                }
            },
            _ => false,
        });
        if exports_value {
            self.push(2309, assignment.span, &[]);
        }
    }

    /// tsc's `checkParameter` rest rule — TS2370 — where the annotation alone
    /// settles it: a primitive or literal type is never an array, and a `?`
    /// adds `undefined` under `strictNullChecks`. oxc drops the `?`, so it is
    /// read from the text between the name and the annotation.
    fn in_context_sensitive_function(&self) -> bool {
        let mut ancestors = self.stack.iter().rev();
        while let Some(kind) = ancestors.next() {
            match kind {
                AstKind::ArrowFunctionExpression(_) => return true,
                AstKind::Function(function) => {
                    return function.is_expression()
                        && !matches!(ancestors.next(), Some(AstKind::MethodDefinition(_)));
                }
                _ => {}
            }
        }
        false
    }

    /// A rest parameter that is not last is parsed as an ordinary one (TS1014),
    /// whose annotation `checkParameter` still judges as a rest's. The parser
    /// settles a keyword; a type reference is left to the checker.
    fn check_demoted_rest_parameter(&mut self, parameter: &oxc_ast::ast::FormalParameter<'_>) {
        if !matches!(parameter.pattern, BindingPattern::BindingIdentifier(_))
            || !self
                .source_text
                .get(parameter.span.start as usize..)
                .is_some_and(|text| text.starts_with("..."))
        {
            return;
        }
        if let Some(annotation) = parameter.type_annotation.as_ref() {
            self.push_rest_type_reference(parameter.span, annotation);
        }
    }

    fn push_rest_type_reference(&mut self, span: Span, annotation: &oxc_ast::ast::TSTypeAnnotation<'_>) {
        if let oxc_ast::ast::TSType::TSTypeReference(reference) = &annotation.type_annotation
            && reference.type_arguments.is_none()
            && let oxc_ast::ast::TSTypeName::IdentifierReference(type_name) = &reference.type_name
            && !self.is_type_parameter_in_scope(&type_name.name)
        {
            self.out.push(ParsedGrammarDiagnostic {
                kind: Kind::RestParameterTypeReference,
                span: text_span_from_oxc_span(span),
                name: Some(type_name.name.to_string()),
            });
        }
    }

    fn check_rest_parameter_type(&mut self, rest: &oxc_ast::ast::FormalParameterRest<'_>) {
        // Unannotated, the type is the initializer's (`...a = 0`), or `any[]`
        // with `undefined` added for `...a?`.
        if rest.type_annotation.is_none() {
            match &rest.rest.argument {
                BindingPattern::AssignmentPattern(assignment)
                    if matches!(
                        assignment.right,
                        Expression::NumericLiteral(_)
                            | Expression::StringLiteral(_)
                            | Expression::BooleanLiteral(_)
                            | Expression::BigIntLiteral(_)
                    ) =>
                {
                    self.push(2370, rest.span, &[]);
                }
                // `...a?` adds `undefined` where the parameter is typed
                // contextually (`assignContextualParameterTypes`): in a function
                // expression, arrow or object-literal method, not a declaration
                // or class member.
                BindingPattern::BindingIdentifier(_)
                    if self.in_context_sensitive_function()
                        && self
                            .source_text
                            .get(rest.span.start as usize..rest.span.end as usize)
                            .is_some_and(|text| text.contains('?')) =>
                {
                    self.out.push(ParsedGrammarDiagnostic {
                        kind: Kind::TsUnderStrictNullChecks(2370),
                        span: text_span_from_oxc_span(rest.span),
                        name: None,
                    });
                }
                _ => {}
            }
            return;
        }
        let BindingPattern::BindingIdentifier(name) = &rest.rest.argument else {
            return;
        };
        let Some(annotation) = rest.type_annotation.as_ref() else {
            return;
        };
        use oxc_ast::ast::TSType as T;
        let never_array = matches!(
            annotation.type_annotation,
            T::TSStringKeyword(_)
                | T::TSNumberKeyword(_)
                | T::TSBooleanKeyword(_)
                | T::TSBigIntKeyword(_)
                | T::TSSymbolKeyword(_)
                | T::TSObjectKeyword(_)
                | T::TSUnknownKeyword(_)
                | T::TSVoidKeyword(_)
                | T::TSUndefinedKeyword(_)
                | T::TSNullKeyword(_)
                | T::TSLiteralType(_)
        );
        if never_array {
            self.push(2370, rest.span, &[]);
            return;
        }
        self.push_rest_type_reference(rest.span, annotation);
        let between = self
            .source_text
            .get(name.span.end as usize..annotation.span.start as usize)
            .unwrap_or("");
        if between.contains('?') {
            self.out.push(ParsedGrammarDiagnostic {
                kind: Kind::TsUnderStrictNullChecks(2370),
                span: text_span_from_oxc_span(rest.span),
                name: None,
            });
        }
    }

    /// The `!` rules of tsc's `checkGrammarVariableDeclaration`: with an
    /// initializer TS1263, without a type TS1264, and anywhere else it is not
    /// a plain variable statement (a loop head, an ambient declaration) TS1255.
    /// oxc reports the first two as well; reporting them here claims its copy.
    fn check_definite_assertion(&mut self, declarator: &oxc_ast::ast::VariableDeclarator<'_>) {
        if !declarator.definite {
            return;
        }
        let in_statement = !matches!(
            self.stack.iter().rev().nth(1),
            Some(AstKind::ForStatement(_) | AstKind::ForInStatement(_) | AstKind::ForOfStatement(_))
        );
        let code = if declarator.init.is_some() {
            1263
        } else if declarator.type_annotation.is_none() {
            1264
        } else if !in_statement || self.ambient_depth > 0 {
            1255
        } else {
            return;
        };
        self.push_at_exclamation(code, declarator.id.span());
    }

    /// tsc's `isValidConstAssertionArgument` (TS1355): `as const` applies to
    /// a literal or to an enum member reference. A member access is left to
    /// the checker's symbols and never reported here.
    fn check_const_assertion(&mut self, ty: &oxc_ast::ast::TSType<'_>, expression: &Expression<'_>) {
        if !is_const_type_reference(ty) {
            return;
        }
        fn valid(expression: &Expression<'_>) -> bool {
            match expression {
                Expression::StringLiteral(_)
                | Expression::NumericLiteral(_)
                | Expression::BigIntLiteral(_)
                | Expression::BooleanLiteral(_)
                | Expression::TemplateLiteral(_)
                | Expression::ArrayExpression(_)
                | Expression::ObjectExpression(_) => true,
                Expression::ParenthesizedExpression(inner) => valid(&inner.expression),
                Expression::UnaryExpression(unary) => match unary.operator {
                    oxc_ast::ast::UnaryOperator::UnaryNegation => {
                        matches!(unary.argument, Expression::NumericLiteral(_) | Expression::BigIntLiteral(_))
                    }
                    oxc_ast::ast::UnaryOperator::UnaryPlus => matches!(unary.argument, Expression::NumericLiteral(_)),
                    _ => false,
                },
                Expression::StaticMemberExpression(_)
                | Expression::ComputedMemberExpression(_)
                | Expression::PrivateFieldExpression(_) => true,
                _ => false,
            }
        }
        if !valid(expression) {
            self.push(1355, expression.span(), &[]);
        }
    }

    fn push_at_exclamation(&mut self, code: u32, name_span: Span) {
        let after = name_span.end as usize;
        if let Some(offset) = self.source_text[after..].find('!') {
            let start = (after + offset) as u32;
            self.push(code, Span::new(start, start + 1), &[]);
        }
    }

    /// tsc's `checkGrammarForDisallowedBlockScopedVariableStatement`: a
    /// block-scoped declaration as the whole body of an `if` or a loop —
    /// through any labels in between (`allowLetAndConstDeclarations` looks
    /// past a label to its parent) — is TS1156. The body of a `with` would be
    /// too, but tsc's `checkWithStatement` never checks it.
    fn check_single_statement_declaration(&mut self, declaration: &oxc_ast::ast::VariableDeclaration<'_>) {
        let keyword = match declaration.kind {
            VariableDeclarationKind::Let => "let",
            VariableDeclarationKind::Const => "const",
            VariableDeclarationKind::Using => "using",
            VariableDeclarationKind::AwaitUsing => "await using",
            _ => return,
        };
        let mut child = declaration.span;
        let mut ancestors = self.stack.iter().rev();
        let mut parent = ancestors.next();
        while let Some(AstKind::LabeledStatement(label)) = parent
            && label.body.span() == child
        {
            child = label.span;
            parent = ancestors.next();
        }
        let is_body = match parent {
            Some(AstKind::IfStatement(statement)) => {
                statement.consequent.span() == child
                    || statement.alternate.as_ref().is_some_and(|alternate| alternate.span() == child)
            }
            Some(AstKind::WhileStatement(statement)) => statement.body.span() == child,
            Some(AstKind::DoWhileStatement(statement)) => statement.body.span() == child,
            Some(AstKind::ForStatement(statement)) => statement.body.span() == child,
            Some(AstKind::ForInStatement(statement)) => statement.body.span() == child,
            Some(AstKind::ForOfStatement(statement)) => statement.body.span() == child,
            _ => false,
        };
        if is_body {
            self.push(1156, declaration.span, &[keyword]);
        }
    }

    /// tsc's `checkGrammarVariableDeclarationList` on a `using` declaration
    /// (oxc reports a `for...in` or ambient one): written directly in a `case`
    /// or `default` clause it is TS1547/TS1548, and an `await using` must sit
    /// where an `await` may (`checkGrammarAwaitOrAwaitUsing`) — TS18054 in a
    /// class static block, TS2852 in a function that is not `async` or in a
    /// namespace.
    fn check_using_declaration_list(&mut self, declaration: &oxc_ast::ast::VariableDeclaration<'_>) {
        let is_await = match declaration.kind {
            VariableDeclarationKind::Using => false,
            VariableDeclarationKind::AwaitUsing => true,
            _ => return,
        };
        let Some(last) = declaration.declarations.last() else {
            return;
        };
        if matches!(self.stack.last(), Some(AstKind::ForInStatement(_)))
            || self.ambient_depth > 0
            || declaration.declare
        {
            return;
        }
        let list = Span::new(declaration.span.start, last.span.end);
        if matches!(self.stack.last(), Some(AstKind::SwitchCase(_))) {
            self.push(if is_await { 1548 } else { 1547 }, list, &[]);
            return;
        }
        if !is_await {
            return;
        }
        let top_level = match self.nearest_function_or_static_block() {
            Some(AstKind::StaticBlock(_)) => {
                self.push(18054, list, &[]);
                return;
            }
            Some(AstKind::Function(function)) if function.r#async => return,
            Some(AstKind::ArrowFunctionExpression(arrow)) if arrow.r#async => return,
            Some(_) => false,
            None => !self.stack.iter().any(|kind| matches!(kind, AstKind::TSModuleDeclaration(_))),
        };
        let start = declaration.span.start;
        let keyword = Span::new(start, start + 5);
        if !top_level {
            self.push(2852, keyword, &[]);
            return;
        }
        if !self.external_module && !self.source_text.contains("import.meta") {
            self.push(2853, keyword, &[]);
        }
        // The module-kind rules; the checker keeps the one that applies.
        self.push(1309, keyword, &[]);
        self.push(2854, keyword, &[]);
    }

    /// tsc's `renamedBindingElementsInTypes`: `{ a: string }` in the parameter
    /// of a signature with no body reads as a type annotation but renames `a`
    /// — TS2842 unless the new name is used (only a `typeof` can use it).
    fn check_renamed_binding_in_signature(&mut self, property: &oxc_ast::ast::BindingProperty<'_>) {
        if property.shorthand {
            return;
        }
        let BindingPattern::BindingIdentifier(name) = &property.value else {
            return;
        };
        let property_name = if property.computed {
            let start = property.span.start as usize;
            let key_end = property.key.span().end as usize;
            let Some(close) = self.source_text[key_end..].find(']') else {
                return;
            };
            std::borrow::Cow::Owned(self.source_text[start..key_end + close + 1].to_string())
        } else {
            let Some(property_name) = property.key.static_name() else {
                return;
            };
            property_name
        };
        let mut in_parameter = false;
        for kind in self.stack.iter().rev() {
            match kind {
                AstKind::FormalParameter(_) | AstKind::FormalParameterRest(_) => in_parameter = true,
                AstKind::ObjectPattern(_)
                | AstKind::ArrayPattern(_)
                | AstKind::BindingProperty(_)
                | AstKind::BindingRestElement(_)
                | AstKind::AssignmentPattern(_)
                | AstKind::FormalParameters(_) => {}
                AstKind::Function(function) if in_parameter && function.body.is_none() => {
                    return self.report_unused_renaming(name, &property_name, function.span);
                }
                AstKind::TSFunctionType(signature) if in_parameter => {
                    return self.report_unused_renaming(name, &property_name, signature.span);
                }
                AstKind::TSConstructorType(signature) if in_parameter => {
                    return self.report_unused_renaming(name, &property_name, signature.span);
                }
                AstKind::TSMethodSignature(signature) if in_parameter => {
                    return self.report_unused_renaming(name, &property_name, signature.span);
                }
                AstKind::TSCallSignatureDeclaration(signature) if in_parameter => {
                    return self.report_unused_renaming(name, &property_name, signature.span);
                }
                AstKind::TSConstructSignatureDeclaration(signature) if in_parameter => {
                    return self.report_unused_renaming(name, &property_name, signature.span);
                }
                _ => return,
            }
        }
    }

    fn report_unused_renaming(
        &mut self,
        name: &oxc_ast::ast::BindingIdentifier<'_>,
        property_name: &str,
        signature_span: Span,
    ) {
        let signature = &self.source_text[signature_span.start as usize..signature_span.end as usize];
        let query = format!("typeof {}", name.name);
        let referenced = signature.match_indices(&query).any(|(at, _)| {
            !signature[at + query.len()..]
                .starts_with(|ch: char| ch.is_alphanumeric() || ch == '_' || ch == '$')
        });
        if !referenced {
            self.push(2842, name.span, &[name.name.as_str(), property_name]);
        }
    }

    /// tsc's `onSuccessfullyResolvedSymbol` for a name read by a parameter's
    /// initializer — or a default inside a destructured parameter — outside
    /// any deferred (function) context: the binding itself is TS2372, a
    /// parameter binding declared after the reference, or a declaration of
    /// the function body (which shares the parameters' scope), TS2373.
    fn check_parameter_initializer_references(&mut self, parameters: &oxc_ast::ast::FormalParameters<'_>) {
        let mut bindings: Vec<(&str, Span)> = Vec::new();
        for parameter in &parameters.items {
            collect_binding_names(&parameter.pattern, &mut bindings);
        }
        if let Some(rest) = &parameters.rest {
            collect_binding_names(&rest.rest.argument, &mut bindings);
        }
        let body_names: Vec<String> = match self.stack.last() {
            Some(AstKind::Function(function)) => function
                .body
                .as_ref()
                .map(|body| body_declared_names(&body.statements))
                .unwrap_or_default(),
            Some(AstKind::ArrowFunctionExpression(arrow)) => body_declared_names(&arrow.body.statements),
            _ => Vec::new(),
        };
        let mut defaults: Vec<(Vec<(&str, Span)>, &Expression<'_>)> = Vec::new();
        for parameter in &parameters.items {
            if let Some(initializer) = parameter.initializer.as_deref() {
                let mut own = Vec::new();
                collect_binding_names(&parameter.pattern, &mut own);
                defaults.push((own, initializer));
            }
            collect_pattern_defaults(&parameter.pattern, &mut defaults);
        }
        if let Some(rest) = &parameters.rest {
            collect_pattern_defaults(&rest.rest.argument, &mut defaults);
        }
        // `useOuterVariableScopeInParameter`: the parameters resolve in the
        // body's scope only when one of them `requiresScopeChange` — here a
        // class expression with a static property, while class fields are not
        // emitted as standard ones.
        let scanned: Vec<_> = defaults
            .into_iter()
            .map(|(own, initializer)| {
                let mut references = EagerReferences::default();
                references.visit_expression(initializer);
                (own, references)
            })
            .collect();
        let reads_body_scope = scanned.iter().any(|(_, references)| references.static_class_property);
        for (own, references) in scanned {
            let Some(&(own_name, _)) = own.first() else { continue };
            for (name, span) in references.found {
                if own.iter().any(|(binding, _)| *binding == name) {
                    self.push(2372, span, &[name.as_str()]);
                } else if bindings.iter().any(|(binding, declared)| *binding == name && declared.start > span.start) {
                    self.push(2373, span, &[own_name, &name]);
                } else if reads_body_scope
                    && !bindings.iter().any(|(binding, _)| *binding == name)
                    && body_names.iter().any(|local| *local == name)
                {
                    self.out.push(ParsedGrammarDiagnostic {
                        kind: Kind::TsUnlessStandardClassFields(2373),
                        span: text_span_from_oxc_span(span),
                        name: Some([own_name, name.as_str()].join("\0")),
                    });
                }
            }
        }
    }

    /// A private name declared on both the static and the instance side —
    /// TS2804 at every declaration of it.
    fn check_private_name_staticness(&mut self, body: &oxc_ast::ast::ClassBody<'_>) {
        use oxc_ast::ast::{ClassElement, PropertyKey};
        let names: Vec<(&str, bool, Span)> = body
            .body
            .iter()
            .filter_map(|element| {
                let (key, is_static) = match element {
                    ClassElement::PropertyDefinition(member) => (&member.key, member.r#static),
                    ClassElement::MethodDefinition(member) => (&member.key, member.r#static),
                    ClassElement::AccessorProperty(member) => (&member.key, member.r#static),
                    _ => return None,
                };
                match key {
                    PropertyKey::PrivateIdentifier(name) => Some((name.name.as_str(), is_static, name.span)),
                    _ => None,
                }
            })
            .collect();
        for (name, _, span) in &names {
            let sides: Vec<bool> = names
                .iter()
                .filter(|(other, _, _)| other == name)
                .map(|(_, is_static, _)| *is_static)
                .collect();
            if sides.contains(&true) && sides.contains(&false) {
                self.push(2804, *span, &[&format!("#{name}")]);
            }
        }
    }

    fn check_self_referencing_annotations(&mut self, statements: &[Statement<'_>]) {
        for (name, span) in self_referencing_annotations(statements) {
            self.push(2502, span, &[name]);
        }
    }

    /// tsc's circular-constraint check for one type parameter list: a
    /// parameter whose base constraint (`computeBaseConstraint`, which looks
    /// through unions, intersections and indexed accesses) is, through other
    /// parameters of the list, itself — TS2313 on the constraint. An indexed
    /// access on the circular parameter itself cannot be resolved (TS2536).
    fn check_circular_constraints(&mut self, declaration: &oxc_ast::ast::TSTypeParameterDeclaration<'_>) {
        use oxc_ast::ast::TSType as T;
        let params = &declaration.params;
        let param_named = |name: &str| params.iter().position(|param| param.name.name == name);
        let literal_key = |index: &T<'_>| match index {
            T::TSLiteralType(literal) => match &literal.literal {
                oxc_ast::ast::TSLiteral::StringLiteral(string) => Some(string.value.to_string()),
                _ => None,
            },
            _ => None,
        };
        // Parameters the base constraint of `ty` resolves through, each with
        // the property keys still to be read from it.
        fn reached<'t, 'a>(
            ty: &'t T<'a>,
            keys: &mut Vec<Option<String>>,
            param_named: &dyn Fn(&str) -> Option<usize>,
            literal_key: &dyn Fn(&T<'_>) -> Option<String>,
            out: &mut Vec<(usize, Vec<Option<String>>)>,
        ) {
            match ty {
                T::TSTypeReference(reference) if reference.type_arguments.is_none() => {
                    if let oxc_ast::ast::TSTypeName::IdentifierReference(name) = &reference.type_name
                        && let Some(index) = param_named(name.name.as_str())
                    {
                        out.push((index, keys.clone()));
                    }
                }
                T::TSUnionType(union) => {
                    for member in &union.types {
                        reached(member, keys, param_named, literal_key, out);
                    }
                }
                T::TSIntersectionType(intersection) => {
                    for member in &intersection.types {
                        reached(member, keys, param_named, literal_key, out);
                    }
                }
                T::TSParenthesizedType(inner) => reached(&inner.type_annotation, keys, param_named, literal_key, out),
                T::TSIndexedAccessType(access) => {
                    keys.push(literal_key(&access.index_type));
                    reached(&access.object_type, keys, param_named, literal_key, out);
                    keys.pop();
                }
                T::TSTypeLiteral(literal) => {
                    let Some(Some(key)) = keys.last().cloned() else {
                        return;
                    };
                    for member in &literal.members {
                        if let oxc_ast::ast::TSSignature::TSPropertySignature(property) = member
                            && property.key.static_name().is_some_and(|name| name == key.as_str())
                            && let Some(annotation) = &property.type_annotation
                        {
                            let read = keys.pop();
                            reached(&annotation.type_annotation, keys, param_named, literal_key, out);
                            keys.extend(read);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut circular = vec![false; params.len()];
        for (start, param) in params.iter().enumerate() {
            let Some(constraint) = param.constraint.as_ref() else {
                continue;
            };
            let mut visited: Vec<(usize, Vec<Option<String>>)> = Vec::new();
            let mut stack = Vec::new();
            reached(constraint, &mut Vec::new(), &param_named, &literal_key, &mut stack);
            while let Some((index, mut keys)) = stack.pop() {
                if index == start {
                    circular[start] = true;
                    break;
                }
                if visited.contains(&(index, keys.clone())) || visited.len() > 64 {
                    continue;
                }
                visited.push((index, keys.clone()));
                if let Some(next) = params[index].constraint.as_ref() {
                    reached(next, &mut keys, &param_named, &literal_key, &mut stack);
                }
            }
            if circular[start] {
                self.push(2313, constraint.span(), &[param.name.name.as_str()]);
            }
        }
        fn indexed_on<'t, 'a>(ty: &'t T<'a>, out: &mut Vec<&'t oxc_ast::ast::TSIndexedAccessType<'a>>) {
            match ty {
                T::TSUnionType(union) => union.types.iter().for_each(|member| indexed_on(member, out)),
                T::TSIntersectionType(intersection) => {
                    intersection.types.iter().for_each(|member| indexed_on(member, out))
                }
                T::TSParenthesizedType(inner) => indexed_on(&inner.type_annotation, out),
                T::TSIndexedAccessType(access) => out.push(access),
                _ => {}
            }
        }
        for (index, param) in params.iter().enumerate() {
            if !circular[index] {
                continue;
            }
            let mut accesses = Vec::new();
            indexed_on(param.constraint.as_ref().expect("circular parameters are constrained"), &mut accesses);
            for access in accesses {
                let T::TSTypeReference(object) = &access.object_type else {
                    continue;
                };
                let oxc_ast::ast::TSTypeName::IdentifierReference(name) = &object.type_name else {
                    continue;
                };
                if object.type_arguments.is_some() || name.name != param.name.name {
                    continue;
                }
                if let Some(key) = literal_key(&access.index_type) {
                    self.push(2536, access.span, &[&format!("\"{key}\""), name.name.as_str()]);
                }
            }
        }
    }

    /// tsc's `checkTypeParameter` on a mapped type's key: its constraint is in
    /// the key's own scope, and resolving the key's base constraint cycles when
    /// the constraint is the key, or a union or intersection with the key among
    /// its members (`computeBaseConstraint`) — TS2313 on the constraint.
    fn check_circular_mapped_key(&mut self, mapped: &oxc_ast::ast::TSMappedType<'_>) {
        fn names_key(ty: &oxc_ast::ast::TSType<'_>, key: &str) -> bool {
            use oxc_ast::ast::TSType as T;
            match ty {
                T::TSTypeReference(reference) if reference.type_arguments.is_none() => matches!(
                    &reference.type_name,
                    oxc_ast::ast::TSTypeName::IdentifierReference(name) if name.name == key
                ),
                T::TSUnionType(union) => union.types.iter().any(|member| names_key(member, key)),
                T::TSIntersectionType(intersection) => {
                    intersection.types.iter().any(|member| names_key(member, key))
                }
                T::TSParenthesizedType(parenthesized) => names_key(&parenthesized.type_annotation, key),
                _ => false,
            }
        }
        let key = mapped.key.name.as_str();
        if names_key(&mapped.constraint, key) {
            self.push(2313, mapped.constraint.span(), &[key]);
        }
    }

    /// tsc's `checkGrammarTypeOperatorNode` for `unique symbol`: only a
    /// `const` variable in a variable statement (TS1332/TS1333/TS1334), a
    /// `static readonly` class property (TS1331), or a `readonly` property
    /// signature (TS1330) may have the type; anywhere else is TS1335.
    fn check_unique_symbol_position(&mut self, operator: &oxc_ast::ast::TSTypeOperator<'_>) {
        if operator.operator != oxc_ast::ast::TSTypeOperatorOperator::Unique {
            return;
        }
        // `checkGrammarTypeOperatorNode`: `unique` takes nothing but `symbol`.
        if !matches!(operator.type_annotation, oxc_ast::ast::TSType::TSSymbolKeyword(_)) {
            self.push(1005, operator.type_annotation.span(), &["symbol"]);
            return;
        }
        let mut ancestors = self.stack.iter().rev();
        let mut parent = ancestors.next();
        while let Some(AstKind::TSParenthesizedType(_)) = parent {
            parent = ancestors.next();
        }
        let owner = match parent {
            Some(AstKind::TSTypeAnnotation(_)) => ancestors.next(),
            _ => None,
        };
        match owner {
            Some(AstKind::VariableDeclarator(declarator)) => {
                let BindingPattern::BindingIdentifier(id) = &declarator.id else {
                    self.push(1333, operator.span, &[]);
                    return;
                };
                let in_statement = !matches!(
                    ancestors.nth(1),
                    Some(AstKind::ForStatement(_) | AstKind::ForInStatement(_) | AstKind::ForOfStatement(_))
                );
                if !in_statement {
                    self.push(1334, operator.span, &[]);
                } else if declarator.kind != VariableDeclarationKind::Const {
                    self.push(1332, id.span, &[]);
                }
            }
            Some(AstKind::PropertyDefinition(property)) => {
                if !property.r#static || !property.readonly {
                    self.push(1331, property.key.span(), &[]);
                }
            }
            Some(AstKind::TSPropertySignature(property)) => {
                if !property.readonly {
                    self.push(1330, property.key.span(), &[]);
                }
            }
            _ => self.push(1335, operator.span, &[]),
        }
    }

    /// tsc's `checkParameter`: an accessibility, `readonly` or `override`
    /// modifier declares a parameter property, which only a constructor with
    /// a body can have — TS2369 on any other parameter list.
    fn check_parameter_property(&mut self, parameter: &oxc_ast::ast::FormalParameter<'_>) {
        if parameter.accessibility.is_none() && !parameter.readonly && !parameter.r#override {
            return;
        }
        let mut ancestors = self.stack.iter().rev();
        ancestors.next();
        let in_constructor_implementation = match (ancestors.next(), ancestors.next()) {
            (Some(AstKind::Function(function)), Some(AstKind::MethodDefinition(method))) => {
                method.kind == oxc_ast::ast::MethodDefinitionKind::Constructor
                    && function.body.is_some()
            }
            _ => false,
        };
        if !in_constructor_implementation {
            self.push(2369, parameter.span, &[]);
        }
    }

    /// tsc's `checkJSDocTypeIsInJsFile`: a JSDoc `T!`, `!T`, `T?` or `?T` in
    /// TypeScript is TS17019 (postfix) or TS17020 (prefix), suggesting the
    /// written `T` — for `?`, with `undefined` added when postfix and both
    /// `null` and `undefined` when prefix.
    fn check_jsdoc_nullability(
        &mut self,
        token: &str,
        postfix: bool,
        span: Span,
        inner: &oxc_ast::ast::TSType<'_>,
    ) {
        let inner_span = inner.span();
        let text = self.source_text[inner_span.start as usize..inner_span.end as usize].trim();
        let meant = match (token, postfix) {
            ("?", _) if matches!(text, "never" | "void") => text.to_string(),
            ("?", true) => format!("{text} | undefined"),
            ("?", false) => format!("{text} | null | undefined"),
            _ => text.to_string(),
        };
        self.push(if postfix { 17019 } else { 17020 }, span, &[token, &meant]);
    }

    /// tsc's `findFirstIllegalDecorator`: TS1206 (TS1249 for a method
    /// overload) on the first decorator of a node that `nodeCanBeDecorated`
    /// rejects.
    fn check_decorator_target(&mut self, decorator: &oxc_ast::ast::Decorator<'_>) {
        let Some(target) = self.stack.last().copied() else {
            return;
        };
        let Some((decorators, legality)) = self.decorator_legality(target, self.stack.len() - 1) else {
            return;
        };
        if decorators.first().map(|first| first.span) != Some(decorator.span) {
            return;
        }
        // `checkGrammarModifiers` names a method without a body an overload.
        let code = match target {
            AstKind::MethodDefinition(method)
                if method.kind == MethodDefinitionKind::Method && method.value.body.is_none() =>
            {
                1249
            }
            _ => 1206,
        };
        if let Some(kind) = legality.unless_legal(code) {
            let start = decorator.span.start;
            self.out.push(ParsedGrammarDiagnostic {
                kind,
                span: text_span_from_oxc_span(Span::new(start, start + 1)),
                name: None,
            });
        }
    }

    /// The decorators of `target`, the node at `index` in the stack, and
    /// whether tsc's `nodeCanBeDecorated` accepts them.
    fn decorator_legality(
        &self,
        target: AstKind<'a>,
        index: usize,
    ) -> Option<(&'a [oxc_ast::ast::Decorator<'a>], DecoratorLegality)> {
        let class_at = |position: Option<usize>| {
            position
                .and_then(|position| self.stack.get(position))
                .is_some_and(|kind| {
                    matches!(kind, AstKind::Class(class)
                        if class.r#type == oxc_ast::ast::ClassType::ClassDeclaration)
                })
        };
        let member_of_declaration = class_at(index.checked_sub(2));
        let private = |key: &oxc_ast::ast::PropertyKey<'_>| {
            matches!(key, oxc_ast::ast::PropertyKey::PrivateIdentifier(_))
        };
        match target {
            AstKind::Class(class) => Some((
                &class.decorators[..],
                DecoratorLegality {
                    legacy: class.r#type == oxc_ast::ast::ClassType::ClassDeclaration,
                    es: true,
                },
            )),
            AstKind::PropertyDefinition(property) => Some((
                &property.decorators[..],
                DecoratorLegality {
                    legacy: member_of_declaration && !private(&property.key),
                    es: property.r#type != oxc_ast::ast::PropertyDefinitionType::TSAbstractPropertyDefinition
                        && !property.declare,
                },
            )),
            AstKind::AccessorProperty(property) => Some((
                &property.decorators[..],
                DecoratorLegality {
                    legacy: member_of_declaration && !private(&property.key),
                    es: property.r#type != oxc_ast::ast::AccessorPropertyType::TSAbstractAccessorProperty,
                },
            )),
            AstKind::MethodDefinition(method) => {
                let has_body = method.value.body.is_some();
                Some((
                    &method.decorators[..],
                    DecoratorLegality {
                        legacy: has_body && member_of_declaration && !private(&method.key),
                        es: has_body,
                    },
                ))
            }
            // tsc's parser reports any decorator on a `this` parameter as
            // TS1433 (`parseParameterWorker`), so the checker never weighs
            // where it sits.
            AstKind::FormalParameter(parameter)
                if matches!(&parameter.pattern, BindingPattern::BindingIdentifier(id) if id.name == "this") =>
            {
                None
            }
            // Parameter decorators are legacy-only: a constructor, method or
            // setter with a body, in a class declaration.
            AstKind::FormalParameter(_) | AstKind::FormalParameterRest(_) => {
                let function = index.checked_sub(2).and_then(|position| self.stack.get(position));
                let owner = index.checked_sub(3).and_then(|position| self.stack.get(position));
                let legacy = match (function, owner) {
                    (Some(AstKind::Function(function)), Some(AstKind::MethodDefinition(method))) => {
                        function.body.is_some()
                            && method.kind != MethodDefinitionKind::Get
                            && class_at(index.checked_sub(5))
                    }
                    _ => false,
                };
                let decorators = match target {
                    AstKind::FormalParameter(parameter) => &parameter.decorators[..],
                    AstKind::FormalParameterRest(parameter) => &parameter.decorators[..],
                    _ => return None,
                };
                Some((decorators, DecoratorLegality { legacy, es: false }))
            }
            _ => None,
        }
    }

    /// `import x = require("m")` inside a namespace — TS1147 on the module
    /// name. An ambient external module (`declare module "m"`) may. Whether it
    /// was reported.
    fn check_import_require_in_namespace(&mut self, declaration: &oxc_ast::ast::TSImportEqualsDeclaration<'_>) -> bool {
        let oxc_ast::ast::TSModuleReference::ExternalModuleReference(reference) = &declaration.module_reference else {
            return false;
        };
        let in_namespace = self.stack.iter().rev().find_map(|kind| match kind {
            AstKind::TSModuleDeclaration(module) => Some(matches!(
                module.id,
                oxc_ast::ast::TSModuleDeclarationName::Identifier(_)
            )),
            AstKind::TSGlobalDeclaration(_) => Some(false),
            _ => None,
        });
        if in_namespace == Some(true) {
            self.push(1147, reference.expression.span, &[]);
            return true;
        }
        false
    }

    /// tsc's `isContainedByNamespace`: the statement's container is a
    /// non-ambient module declaration.
    fn is_contained_by_namespace(&self) -> bool {
        let mut ancestors = self.ancestors_as_tsc();
        matches!(ancestors.next(), Some(AstKind::TSModuleBlock(_)))
            && matches!(
                ancestors.next(),
                Some(AstKind::TSModuleDeclaration(module))
                    if matches!(module.id, oxc_ast::ast::TSModuleDeclarationName::Identifier(_))
            )
    }

    /// `assert { … }` in place of `with { … }` — TS2880 on the keyword.
    fn check_import_assertion(&mut self, clause: &oxc_ast::ast::WithClause<'_>) {
        if clause.keyword == oxc_ast::ast::WithClauseKeyword::Assert
            && let Some(start) = self.source_text[..clause.span.end as usize].rfind("assert")
        {
            let start = start as u32;
            self.push(2880, Span::new(start, start + 6), &[]);
        }
    }

    /// tsc's `checkImportAttributes`: TS2823 on a `with { … }` clause, which
    /// the checker keeps unless the `module` option supports attributes. A
    /// type-only import or export whose one attribute is a valid
    /// `resolution-mode` is exempt.
    fn check_import_attributes_supported(&mut self, clause: &oxc_ast::ast::WithClause<'_>, type_only: bool) {
        if clause.keyword != oxc_ast::ast::WithClauseKeyword::With {
            return;
        }
        if type_only
            && let [attribute] = clause.with_entries.as_slice()
            && attribute.key.as_arena_str().as_str() == "resolution-mode"
        {
            if matches!(attribute.value.value.as_str(), "import" | "require") {
                return;
            }
            // `getResolutionModeOverride`, reporting for a type-only import.
            if matches!(attribute.key, oxc_ast::ast::ImportAttributeKey::StringLiteral(_)) {
                self.push(1453, attribute.value.span, &[]);
            }
        }
        // The clause's span starts after its keyword.
        if let Some(start) = self.source_text[..clause.span.start as usize].rfind("with") {
            self.push(2823, Span::new(start as u32, clause.span.end), &[]);
        }
    }

    /// ECMAScript module syntax that `verbatimModuleSyntax` rejects in a file
    /// emitted as CommonJS; the checker decides the options and the file's
    /// format. A dynamic import (`checkGrammarImportCallExpression`, marked
    /// `import`: it keys on the `module` option alone) and an `export default`
    /// expression (`checkExportAssignment`) are TS1295; a top-level `export`
    /// on a value declaration is TS1287 (`checkGrammarModifiers`, and
    /// `checkModuleDeclaration` for an instantiated namespace).
    fn check_commonjs_module_syntax(&mut self, kind: &AstKind<'a>) {
        let top_level =
            self.ambient_depth == 0 && matches!(self.stack.last(), Some(AstKind::Program(_)));
        match kind {
            AstKind::ImportExpression(import) => self.push(1295, import.span, &["import"]),
            // `checkAliasSymbol`: each value binding of an import or export
            // declaration, marked `alias` (a JavaScript file is exempt).
            AstKind::ImportDeclaration(import) if self.ambient_depth == 0 && !import.import_kind.is_type() => {
                use oxc_ast::ast::ImportDeclarationSpecifier as Specifier;
                for specifier in import.specifiers.iter().flatten() {
                    let span = match specifier {
                        Specifier::ImportSpecifier(specifier) if specifier.import_kind.is_type() => continue,
                        Specifier::ImportSpecifier(specifier) => specifier.span,
                        Specifier::ImportDefaultSpecifier(specifier) => specifier.local.span,
                        Specifier::ImportNamespaceSpecifier(specifier) => specifier.local.span,
                    };
                    self.push(1295, span, &["alias"]);
                }
            }
            AstKind::ExportNamedDeclaration(export)
                if self.ambient_depth == 0 && export.declaration.is_none() && !export.export_kind.is_type() =>
            {
                for specifier in &export.specifiers {
                    if !specifier.export_kind.is_type() {
                        self.push(1295, specifier.span, &["alias"]);
                    }
                }
            }
            AstKind::ExportNamedDeclaration(export)
                if top_level && export.declaration.as_ref().is_some_and(declares_runtime_value) =>
            {
                let start = export.span.start;
                self.push(1287, Span::new(start, start + 6), &[]);
            }
            AstKind::ExportDefaultDeclaration(export) if top_level => match &export.declaration {
                oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                    if !function.declare {
                        let start = export.span.start;
                        self.push(1287, Span::new(start, start + 6), &[]);
                    }
                }
                oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    if !class.declare {
                        let start = export.span.start;
                        self.push(1287, Span::new(start, start + 6), &[]);
                    }
                }
                oxc_ast::ast::ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
                _ => self.push(1295, export.span, &[]),
            },
            _ => {}
        }
    }

    /// Whether the node being entered is a statement of a source file or a
    /// namespace body — where tsc's `checkGrammarModuleElementContext` allows
    /// `export` and `declare`. Anywhere else they are TS1184.
    fn at_module_element_level(&self) -> bool {
        let mut ancestors = self.stack.iter().rev();
        let mut parent = ancestors.next();
        if matches!(parent, Some(AstKind::ExportNamedDeclaration(_))) {
            parent = ancestors.next();
        }
        matches!(parent, None | Some(AstKind::Program(_) | AstKind::TSModuleBlock(_)))
    }

    /// tsc's `checkImportDeclaration` names a namespace as a legal place only
    /// outside JavaScript, which has none.
    fn misplaced_import_code(&self) -> u32 {
        if self.javascript { 1473 } else { 1232 }
    }

    fn misplaced_export_code(&self) -> u32 {
        if self.javascript { 1474 } else { 1233 }
    }

    /// tsc's `checkClassForStaticPropertyNameConflicts`: a static member named
    /// like one of `Function`'s own properties — TS2699, which the checker
    /// keeps only when `useDefineForClassFields` is off — and the static
    /// `prototype` of `checkObjectTypeForDuplicateDeclarations`, which it
    /// always keeps. Neither is checked in an ambient context.
    fn check_static_function_property_names(&mut self, body: &oxc_ast::ast::ClassBody<'_>) {
        use oxc_ast::ast::ClassElement;
        if self.ambient_depth > 0 {
            return;
        }
        // tsc names an anonymous class expression by the variable it
        // initializes (`const E = class {}` is `E`).
        let class_name = match self.stack.last() {
            Some(AstKind::Class(class)) => match class.id.as_ref() {
                Some(id) => id.name.to_string(),
                None => match self.stack.iter().rev().nth(1) {
                    Some(AstKind::VariableDeclarator(declarator)) => match &declarator.id {
                        BindingPattern::BindingIdentifier(id) => id.name.to_string(),
                        _ => "(Anonymous class)".to_string(),
                    },
                    _ => "(Anonymous class)".to_string(),
                },
            },
            _ => return,
        };
        for element in &body.body {
            let (key, is_static) = match element {
                ClassElement::PropertyDefinition(member) => (&member.key, member.r#static),
                ClassElement::MethodDefinition(member) => (&member.key, member.r#static),
                ClassElement::AccessorProperty(member) => (&member.key, member.r#static),
                _ => continue,
            };
            if !is_static {
                continue;
            }
            let Some(name) = key.static_name() else {
                continue;
            };
            if matches!(name.as_ref(), "name" | "length" | "caller" | "arguments" | "prototype") {
                self.push(2699, key.span(), &[&name, &class_name]);
            }
        }
    }

    /// `in`/`out` on a type alias's type parameter where the alias does not
    /// declare an object, function, constructor or mapped type — TS2637
    /// (`checkTypeParameterDeferred`). Elsewhere they are TS1274, which
    /// [`Self::check_type_parameter_modifiers`] reports.
    fn check_variance_modifier_owner(&mut self, parameter: &oxc_ast::ast::TSTypeParameter<'_>) {
        if !parameter.r#in && !parameter.out {
            return;
        }
        let owner = self.stack.iter().rev().nth(1);
        if let Some(AstKind::TSTypeAliasDeclaration(alias)) = owner {
            if alias_is_not_anonymous(alias, self.alias_file_scope()) {
                self.push(2637, parameter.span, &[]);
            }
        }
    }

    /// The file's top-level statements, when the type alias owning the type
    /// parameter being entered is declared among them.
    fn alias_file_scope(&self) -> Option<&'a [Statement<'a>]> {
        let mut ancestors = self.stack.iter().rev().skip(2).copied();
        let mut parent = ancestors.next();
        if matches!(parent, Some(AstKind::ExportNamedDeclaration(_))) {
            parent = ancestors.next();
        }
        match parent {
            Some(AstKind::Program(program)) => Some(&program.body[..]),
            _ => None,
        }
    }

    /// The declaration half of tsc's `checkGrammarForInOrForOfStatement`: one
    /// declaration only (TS1091/TS1188), no initializer (TS1189/TS1190), no
    /// type annotation (TS2404/TS2483) — the first that applies.
    fn check_for_in_or_of_declaration(&mut self, left: &oxc_ast::ast::ForStatementLeft<'_>, is_in: bool) {
        let oxc_ast::ast::ForStatementLeft::VariableDeclaration(declaration) = left else {
            return;
        };
        if self.ambient_depth > 0 {
            return;
        }
        let declarators = &declaration.declarations;
        if declarators.len() > 1 {
            let start = declarators[1].span.start;
            let end = first_token_end(self.source_text, start as usize) as u32;
            self.push(if is_in { 1091 } else { 1188 }, Span::new(start, end), &[]);
            return;
        }
        let Some(first) = declarators.first() else {
            return;
        };
        if first.init.is_some() {
            self.push(if is_in { 1189 } else { 1190 }, first.id.span(), &[]);
        } else if first.type_annotation.is_some() {
            self.push(if is_in { 2404 } else { 2483 }, first.id.span(), &[]);
        }
    }

    /// tsc's `checkGrammarForInvalidDynamicName`: a computed name that is
    /// neither a literal nor an entity name (`a`, `a.b`) cannot be bound —
    /// TS1166 on a class property, TS1169/TS1170 on an interface or type
    /// literal member, reported on the bracketed name.
    fn check_dynamic_property_name(&mut self, code: u32, key: &oxc_ast::ast::PropertyKey<'_>) {
        if let Some(span) = self.dynamic_property_name_span(key) {
            self.push(code, span, &[]);
        }
    }

    /// The bracketed name of a computed key that is neither a literal nor an
    /// entity name.
    fn dynamic_property_name_span(&self, key: &oxc_ast::ast::PropertyKey<'_>) -> Option<Span> {
        let expression = key.as_expression()?.without_parentheses();
        if is_literal_name(expression) || is_entity_name_expression(expression) {
            return None;
        }
        Some(self.member_name_span(key, true))
    }

    /// A member name as tsc's `getErrorSpanForNode` reports it: a computed
    /// name includes its brackets, which oxc's key span leaves out.
    fn member_name_span(&self, key: &oxc_ast::ast::PropertyKey<'_>, computed: bool) -> Span {
        let key_span = key.span();
        if !computed {
            return key_span;
        }
        let open = self.source_text[..key_span.start as usize]
            .rfind('[')
            .map_or(key_span.start, |open| open as u32);
        let close = self.source_text[key_span.end as usize..]
            .find(']')
            .map_or(key_span.end, |offset| key_span.end + offset as u32 + 1);
        Span::new(open, close)
    }

    /// tsc's `checkGrammarProperty`: a `[k in T]` property name is a mapped
    /// type written among other members, reported on the parent's first
    /// member (TS7061). Returns whether it was, since tsc then skips the
    /// dynamic-name check.
    fn check_mapped_type_member(&mut self, key: &oxc_ast::ast::PropertyKey<'_>) -> bool {
        use oxc_ast::ast::{ClassElement, Expression, TSSignature};
        let is_in_expression = match key.as_expression() {
            Some(Expression::BinaryExpression(binary)) => {
                binary.operator == oxc_syntax::operator::BinaryOperator::In
            }
            Some(Expression::PrivateInExpression(_)) => true,
            _ => false,
        };
        if !is_in_expression {
            return false;
        }
        let signature_span = |signature: &TSSignature<'_>| match signature {
            TSSignature::TSPropertySignature(member) => self.member_name_span(&member.key, member.computed),
            TSSignature::TSMethodSignature(member) => self.member_name_span(&member.key, member.computed),
            other => other.span(),
        };
        let first_member = match self.stack.last() {
            Some(AstKind::ClassBody(body)) => body.body.first().map(|element| match element {
                ClassElement::MethodDefinition(member) => self.member_name_span(&member.key, member.computed),
                ClassElement::PropertyDefinition(member) => self.member_name_span(&member.key, member.computed),
                ClassElement::AccessorProperty(member) => self.member_name_span(&member.key, member.computed),
                other => other.span(),
            }),
            Some(AstKind::TSInterfaceBody(body)) => body.body.first().map(signature_span),
            Some(AstKind::TSTypeLiteral(literal)) => literal.members.first().map(signature_span),
            _ => None,
        };
        let Some(span) = first_member else {
            return false;
        };
        self.push(7061, span, &[]);
        true
    }

    /// tsc's `checkGrammarBreakOrContinueStatement`.
    fn check_jump(&mut self, span: Span, label: Option<&str>, is_continue: bool) {
        if self.ambient_reported_statements.contains(&span.start) {
            return;
        }
        for kind in self.stack.iter().rev() {
            if is_function_like(kind) || matches!(kind, AstKind::StaticBlock(_)) {
                self.push(1107, span, &[]);
                return;
            }
            match kind {
                AstKind::LabeledStatement(labeled)
                    if label.is_some_and(|label| labeled.label.name == label) =>
                {
                    if is_continue && !is_iteration_statement(&labeled.body, true) {
                        self.push(1115, span, &[]);
                    }
                    return;
                }
                AstKind::SwitchStatement(_) if !is_continue && label.is_none() => return,
                _ if label.is_none() && is_iteration_kind(kind) => return,
                _ => {}
            }
        }
        let code = match (label.is_some(), is_continue) {
            (true, false) => 1116,
            (true, true) => 1115,
            (false, false) => 1105,
            (false, true) => 1104,
        };
        self.push(code, span, &[]);
    }

    fn check_labeled_statement(&mut self, labeled: &oxc_ast::ast::LabeledStatement<'_>) {
        let name = labeled.label.name.as_str();
        if self.ambient_depth == 0 {
            for kind in self.stack.iter().rev() {
                if is_function_like(kind) {
                    break;
                }
                if let AstKind::LabeledStatement(outer) = kind
                    && outer.label.name == name
                {
                    self.push(1114, labeled.label.span, &[name]);
                    break;
                }
            }
        }
        // The binder leaves a label no `break`/`continue` targets unreachable
        // and `checkLabeledStatement` reports it — TS7028, an error only under
        // an explicit `allowUnusedLabels: false`, which the checker gates.
        if !label_is_referenced(&labeled.body, name) {
            self.push(7028, labeled.label.span, &[]);
        }
        // The binder's `checkStrictModeLabeledStatement`.
        let is_declaration = matches!(
            labeled.body,
            Statement::VariableDeclaration(_)
                | Statement::FunctionDeclaration(_)
                | Statement::ClassDeclaration(_)
                | Statement::TSInterfaceDeclaration(_)
                | Statement::TSTypeAliasDeclaration(_)
                | Statement::TSEnumDeclaration(_)
                | Statement::TSModuleDeclaration(_)
                | Statement::TSGlobalDeclaration(_)
                | Statement::ImportDeclaration(_)
                | Statement::TSImportEqualsDeclaration(_)
                | Statement::ExportAllDeclaration(_)
                | Statement::ExportDefaultDeclaration(_)
                | Statement::ExportNamedDeclaration(_)
                | Statement::TSExportAssignment(_)
                | Statement::TSNamespaceExportDeclaration(_)
        );
        if is_declaration {
            self.push(1344, labeled.label.span, &[]);
        }
    }

    fn check_with_statement(&mut self, statement: &oxc_ast::ast::WithStatement<'_>) {
        let start = statement.span.start;
        self.push(1101, Span::new(start, start + 4), &[]);
        // tsc spans `with (…)` up to the statement's full start, which is
        // just past the closing parenthesis.
        let after_object = statement.object.span().end as usize;
        if let Some(offset) = self.source_text[after_object..].find(')') {
            let end = (after_object + offset + 1) as u32;
            self.push(2410, Span::new(start, end), &[]);
        }
    }

    /// The binder's `checkStrictModeEvalOrArguments`.
    fn check_eval_or_arguments(&mut self, name: &str, span: Span) {
        if name != "eval" && name != "arguments" {
            return;
        }
        let code = if self.in_class() {
            1210
        } else if self.external_module {
            1215
        } else {
            1100
        };
        self.push(code, span, &[name]);
    }

    /// Whether a binding identifier sits where the binder checks its name
    /// for `eval`/`arguments`: a variable, catch, or parameter binding, or a
    /// function name. A parameter or function name in an ambient context is
    /// exempt; a destructured binding never is.
    fn is_strict_checked_binding(&self) -> bool {
        let mut in_pattern = false;
        for kind in self.stack.iter().rev() {
            match kind {
                AstKind::ObjectPattern(_)
                | AstKind::ArrayPattern(_)
                | AstKind::BindingProperty(_)
                | AstKind::BindingRestElement(_)
                | AstKind::AssignmentPattern(_) => in_pattern = true,
                AstKind::VariableDeclarator(_) | AstKind::CatchParameter(_) => return true,
                AstKind::FormalParameter(_) | AstKind::FormalParameterRest(_) => {
                    return in_pattern || self.ambient_depth == 0;
                }
                AstKind::Function(_) => return !in_pattern && self.ambient_depth == 0,
                _ => return false,
            }
        }
        false
    }

    /// The binder's `checkContextualIdentifier` for strict-mode reserved words.
    fn check_contextual_identifier(&mut self, name: &str, span: Span) {
        if self.ambient_depth > 0 {
            return;
        }
        if !matches!(
            name,
            "implements"
                | "interface"
                | "let"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "static"
                | "yield"
        ) {
            return;
        }
        // An export specifier's names are identifier names to tsc.
        if matches!(self.stack.last(), Some(AstKind::ExportSpecifier(_))) {
            return;
        }
        let code = if self.in_class() {
            1213
        } else if self.external_module {
            1214
        } else {
            1212
        };
        self.push(code, span, &[name]);
    }

    /// `checkTypeParameter`'s circularity: a default that reads its own
    /// declaration without supplying this parameter needs the default being
    /// resolved (`getResolvedTypeParameterDefault`) — TS2716. An object
    /// literal's members and a signature are resolved later, so a reference
    /// there does not count.
    fn check_circular_type_parameter_defaults(
        &mut self,
        name: &str,
        type_parameters: Option<&oxc_ast::ast::TSTypeParameterDeclaration<'_>>,
    ) {
        let Some(type_parameters) = type_parameters else {
            return;
        };
        for (index, parameter) in type_parameters.params.iter().enumerate() {
            if let Some(default) = &parameter.default
                && default_reads_own_declaration(default, name, index)
            {
                self.push(2716, default.span(), &[parameter.name.name.as_str()]);
            }
        }
    }

    fn check_reserved_type_name(&mut self, code: u32, name: &oxc_ast::ast::BindingIdentifier<'_>) {
        if matches!(
            name.name.as_str(),
            "any"
                | "unknown"
                | "never"
                | "number"
                | "bigint"
                | "boolean"
                | "string"
                | "symbol"
                | "void"
                | "object"
                | "undefined"
        ) {
            self.push(code, name.span, &[name.name.as_str()]);
        }
    }

    /// tsc's `isInParameterInitializerBeforeContainingFunction`.
    fn is_in_parameter_initializer(&self, node_span: Span) -> bool {
        let mut child_span = node_span;
        let mut in_binding_initializer = false;
        for kind in self.stack.iter().rev() {
            if is_function_like(kind) {
                return false;
            }
            match kind {
                AstKind::FormalParameter(parameter) => {
                    if in_binding_initializer
                        || parameter.initializer.as_ref().is_some_and(|init| init.span() == child_span)
                    {
                        return true;
                    }
                }
                AstKind::AssignmentPattern(pattern) if pattern.right.span() == child_span => {
                    in_binding_initializer = true;
                }
                _ => {}
            }
            child_span = kind.span();
        }
        false
    }

    fn nearest_function_or_static_block(&self) -> Option<AstKind<'a>> {
        self.stack
            .iter()
            .rev()
            .find(|kind| is_function_like(kind) || matches!(kind, AstKind::StaticBlock(_)))
            .copied()
    }

    fn check_yield(&mut self, expression: &oxc_ast::ast::YieldExpression<'_>) {
        if !self.in_yield_context(expression.span) {
            let start = expression.span.start;
            self.push(1163, Span::new(start, start + 5), &[]);
        }
        if self.is_in_parameter_initializer(expression.span) {
            self.push(2523, expression.span, &[]);
            self.check_parameter_initializer_yield_type(expression);
        }
    }

    /// The rest of tsc's `checkYieldExpression` for `function* f(a = yield)`:
    /// the generator has no return annotation, so the `yield` is `any`, and
    /// with nothing contextually typing it that is TS7057 (decided by the
    /// checker under `noImplicitAny`). Only a parameter's own initializer is
    /// judged; its annotation is the contextual type.
    fn check_parameter_initializer_yield_type(&mut self, expression: &oxc_ast::ast::YieldExpression<'_>) {
        let Some(AstKind::FormalParameter(parameter)) = self.stack.last() else {
            return;
        };
        let untyped = match &parameter.type_annotation {
            None => true,
            Some(annotation) => {
                matches!(annotation.type_annotation, oxc_ast::ast::TSType::TSAnyKeyword(_))
            }
        };
        if !untyped || !parameter.initializer.as_ref().is_some_and(|init| init.span() == expression.span) {
            return;
        }
        let unannotated_generator = matches!(
            self.nearest_function_or_static_block(),
            Some(AstKind::Function(function)) if function.generator && function.return_type.is_none()
        );
        if unannotated_generator {
            self.push(7057, expression.span, &[]);
        }
    }

    /// tsc's `NodeFlagsYieldContext`: a generator's parameters and body are
    /// parsed in it, but the parser clears it for a class field's initializer,
    /// a static block and an enum's members, even inside a generator.
    fn in_yield_context(&self, node_span: Span) -> bool {
        let mut child_span = node_span;
        for kind in self.stack.iter().rev() {
            match kind {
                AstKind::Function(function) => return function.generator,
                AstKind::ArrowFunctionExpression(_) | AstKind::StaticBlock(_) | AstKind::TSEnumMember(_) => {
                    return false;
                }
                AstKind::PropertyDefinition(property)
                    if property.value.as_ref().is_some_and(|value| value.span() == child_span) =>
                {
                    return false;
                }
                AstKind::AccessorProperty(property)
                    if property.value.as_ref().is_some_and(|value| value.span() == child_span) =>
                {
                    return false;
                }
                _ => {}
            }
            child_span = kind.span();
        }
        false
    }

    fn check_await(&mut self, expression: &oxc_ast::ast::AwaitExpression<'_>) {
        if matches!(self.nearest_function_or_static_block(), Some(AstKind::StaticBlock(_))) {
            self.push(18037, expression.span, &[]);
        }
        if self.is_in_parameter_initializer(expression.span) {
            self.push(2524, expression.span, &[]);
        }
    }

    /// `checkGrammarForInOrForOfStatement`: outside an await context a bare
    /// `async` before `of` reads as the start of an async arrow — TS1106.
    fn check_for_of_async_target(&mut self, statement: &oxc_ast::ast::ForOfStatement<'_>) {
        let oxc_ast::ast::ForStatementLeft::AssignmentTargetIdentifier(identifier) = &statement.left else {
            return;
        };
        if identifier.name != "async" {
            return;
        }
        let await_context = match self.nearest_function_or_static_block() {
            Some(AstKind::Function(function)) => function.r#async,
            Some(AstKind::ArrowFunctionExpression(arrow)) => arrow.r#async,
            Some(_) => true,
            None => self.external_module,
        };
        // `for ((async) of …)` is a parenthesized expression, not the name.
        let head = &self.source_text[statement.span.start as usize..identifier.span.start as usize];
        if !await_context && head.matches('(').count() == 1 {
            self.push(1106, identifier.span, &[]);
        }
    }

    fn check_for_await(&mut self, statement: &oxc_ast::ast::ForOfStatement<'_>) {
        // A static block's body is parsed in an await context, so the loop is
        // not TS1103 there; `checkForOfStatement` rejects it as TS18038.
        let code = match self.nearest_function_or_static_block() {
            Some(AstKind::StaticBlock(_)) => 18038,
            Some(AstKind::Function(function)) if !function.r#async => 1103,
            Some(AstKind::ArrowFunctionExpression(arrow)) if !arrow.r#async => 1103,
            _ => return,
        };
        let from = statement.span.start as usize + 3;
        if let Some(offset) = self.source_text[from..].find("await") {
            let start = (from + offset) as u32;
            self.push(code, Span::new(start, start + 5), &[]);
        }
    }

    /// tsc's `checkGrammarMetaProperty`: `new.<name>` must be `new.target`,
    /// and `import.<name>` must be `import.meta` (or `import.defer` called).
    fn check_meta_property_name(&mut self, meta: &oxc_ast::ast::MetaProperty<'_>) {
        let name = meta.property.name.as_str();
        match meta.meta.name.as_str() {
            "new" if name != "target" => self.push(17012, meta.property.span, &[name, "new", "target"]),
            "import" if name != "meta" => {
                let is_callee = matches!(self.stack.last(),
                    Some(AstKind::CallExpression(call)) if call.callee.span() == meta.span);
                if name == "defer" {
                    if !is_callee {
                        self.push(1005, Span::new(meta.span.end, meta.span.end), &["("]);
                    }
                } else if is_callee {
                    self.push(18061, meta.property.span, &[name]);
                } else {
                    self.push(17012, meta.property.span, &[name, "import", "meta"]);
                }
            }
            _ => {}
        }
    }

    fn check_new_target(&mut self, meta: &oxc_ast::ast::MetaProperty<'_>) {
        if meta.meta.name != "new" || meta.property.name != "target" {
            return;
        }
        let allowed = match self.this_container(meta.span) {
            ThisContainer::ClassMethod(method) => method.kind == MethodDefinitionKind::Constructor,
            ThisContainer::Function => true,
            _ => false,
        };
        if !allowed {
            self.push(17013, meta.span, &["new.target"]);
        }
    }

    /// tsc's `getThisType` failure — TS2526.
    fn check_this_type(&mut self, span: Span) {
        let allowed = match self.this_container(span) {
            ThisContainer::ClassMethod(method) => {
                !method.r#static
                    && (method.kind != MethodDefinitionKind::Constructor
                        || method.value.body.as_ref().is_some_and(|body| {
                            body.span.start <= span.start && span.end <= body.span.end
                        }))
            }
            ThisContainer::ClassProperty { is_static } => !is_static,
            ThisContainer::Signature { in_interface, is_static } => in_interface && !is_static,
            ThisContainer::Function | ThisContainer::ObjectMethod | ThisContainer::Other => false,
        };
        if !allowed {
            self.push(2526, span, &[]);
        }
    }

    fn record_infer_declaration(&mut self, infer: &oxc_ast::ast::TSInferType<'a>) {
        let mut child_span = infer.span;
        let owner = self.stack.iter().rev().find_map(|kind| {
            if let AstKind::TSConditionalType(conditional) = kind
                && conditional.extends_type.span() == child_span
            {
                return Some(conditional.span.start);
            }
            child_span = kind.span();
            None
        });
        let Some(owner) = owner else {
            return;
        };
        let parameter = &infer.type_parameter;
        let constraint = parameter.constraint.as_ref().map(|constraint| {
            let span = constraint.span();
            &self.source_text[span.start as usize..span.end as usize]
        });
        self.infer_declarations
            .push((owner, parameter.name.name.as_str(), parameter.name.span, constraint));
    }

    /// tsc's `checkInferType`: the `infer` declarations of one name merge
    /// into one type parameter, and two that constrain it differently are
    /// TS2838 at every declaration.
    fn check_infer_constraints(&mut self, owner: u32) {
        let declarations: Vec<_> = self
            .infer_declarations
            .iter()
            .filter(|(start, ..)| *start == owner)
            .copied()
            .collect();
        self.infer_declarations.retain(|(start, ..)| *start != owner);
        let mut reported: Vec<&str> = Vec::new();
        for (_, name, _, _) in &declarations {
            if reported.contains(name) {
                continue;
            }
            reported.push(name);
            let same: Vec<_> = declarations.iter().filter(|(_, other, ..)| other == name).collect();
            let normalize = |text: &str| text.split_whitespace().collect::<String>();
            let mut constraints = same.iter().filter_map(|(.., constraint)| constraint.map(normalize));
            let Some(first) = constraints.next() else {
                continue;
            };
            if constraints.all(|constraint| constraint == first) {
                continue;
            }
            for (_, _, span, _) in &same {
                self.push(2838, *span, &[name]);
            }
        }
    }

    fn check_infer_type(&mut self, span: Span) {
        let mut child_span = span;
        for kind in self.stack.iter().rev() {
            if let AstKind::TSConditionalType(conditional) = kind
                && conditional.extends_type.span() == child_span
            {
                return;
            }
            child_span = kind.span();
        }
        self.push(1338, span, &[]);
    }

    /// tsc's `checkCatchClause`: a block-scoped variable in the catch block
    /// that redeclares the caught binding — TS2492.
    fn check_catch_clause(&mut self, clause: &oxc_ast::ast::CatchClause<'_>) {
        let Some(parameter) = clause.param.as_ref() else {
            return;
        };
        if parameter.type_annotation.is_some() {
            return;
        }
        let mut caught = Vec::new();
        collect_binding_names(&parameter.pattern, &mut caught);
        let mut reported = Vec::new();
        for statement in &clause.body.body {
            let Statement::VariableDeclaration(declaration) = statement else {
                continue;
            };
            if declaration.kind == VariableDeclarationKind::Var {
                continue;
            }
            for declarator in &declaration.declarations {
                let mut names = Vec::new();
                collect_binding_names(&declarator.id, &mut names);
                for (name, span) in names {
                    if caught.iter().any(|(caught, _)| *caught == name)
                        && !reported.contains(&name)
                    {
                        self.push(2492, span, &[name]);
                        reported.push(name);
                    }
                }
            }
        }
    }

    /// tsc's `checkGrammarNameInLetOrConstDeclarations`, reached only when
    /// `checkGrammarVariableDeclaration` found nothing earlier to report.
    fn check_let_name(&mut self, declarator: &oxc_ast::ast::VariableDeclarator<'_>) {
        if declarator.kind == VariableDeclarationKind::Var {
            return;
        }
        let in_for = matches!(
            self.stack.iter().rev().nth(1),
            Some(AstKind::ForInStatement(_) | AstKind::ForOfStatement(_))
        );
        if !in_for && self.ambient_depth == 0 && declarator.init.is_none() {
            let is_pattern = !matches!(declarator.id, BindingPattern::BindingIdentifier(_));
            if is_pattern || declarator.kind != VariableDeclarationKind::Let {
                return;
            }
        }
        let mut names = Vec::new();
        collect_binding_names(&declarator.id, &mut names);
        for (name, span) in names {
            if name == "let" {
                self.push(2480, span, &[]);
            }
        }
    }
}

/// The defaults written inside a destructuring pattern, each with the names
/// its own target binds.
fn collect_pattern_defaults<'n, 'a>(
    pattern: &'n BindingPattern<'a>,
    out: &mut Vec<(Vec<(&'n str, Span)>, &'n Expression<'a>)>,
) {
    match pattern {
        BindingPattern::BindingIdentifier(_) => {}
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                // A computed key is read in the parameter scope like a default.
                if property.computed
                    && let Some(key) = property.key.as_expression()
                {
                    let mut own = Vec::new();
                    collect_binding_names(&property.value, &mut own);
                    out.push((own, key));
                }
                collect_pattern_defaults(&property.value, out);
            }
            if let Some(rest) = &object.rest {
                collect_pattern_defaults(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                collect_pattern_defaults(element, out);
            }
            if let Some(rest) = &array.rest {
                collect_pattern_defaults(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assignment) => {
            let mut own = Vec::new();
            collect_binding_names(&assignment.left, &mut own);
            out.push((own, &assignment.right));
            collect_pattern_defaults(&assignment.left, out);
        }
    }
}

/// The names a function body declares at its top level.
fn body_declared_names(statements: &[Statement<'_>]) -> Vec<String> {
    let mut names = Vec::new();
    for statement in statements {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                for declarator in &declaration.declarations {
                    let mut bound = Vec::new();
                    collect_binding_names(&declarator.id, &mut bound);
                    names.extend(bound.into_iter().map(|(name, _)| name.to_string()));
                }
            }
            Statement::FunctionDeclaration(function) => {
                names.extend(function.id.as_ref().map(|id| id.name.to_string()));
            }
            Statement::ClassDeclaration(class) => names.extend(class.id.as_ref().map(|id| id.name.to_string())),
            _ => {}
        }
    }
    names
}

fn collect_binding_names<'n>(pattern: &'n BindingPattern<'_>, out: &mut Vec<(&'n str, Span)>) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => {
            out.push((identifier.name.as_str(), identifier.span));
        }
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                collect_binding_names(&property.value, out);
            }
            if let Some(rest) = &object.rest {
                collect_binding_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                collect_binding_names(element, out);
            }
            if let Some(rest) = &array.rest {
                collect_binding_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assignment) => collect_binding_names(&assignment.left, out),
    }
}

/// The end of the bracketed run starting at `start`, or `start` when it never
/// closes.
fn matching_bracket_end(text: &str, start: usize) -> usize {
    let mut depth = 0usize;
    for (offset, byte) in text.as_bytes()[start..].iter().enumerate() {
        match byte {
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return start + offset + 1;
                }
            }
            _ => {}
        }
    }
    start
}

/// Whether a `break` or `continue` under `body` targets `name`, as the
/// binder's active-label list sees it: a nested function or static block
/// starts a fresh list, and a nested label of the same name takes the jumps
/// under it.
fn label_is_referenced(body: &Statement<'_>, name: &str) -> bool {
    struct LabelReferences<'n> {
        name: &'n str,
        found: bool,
    }

    impl<'a> Visit<'a> for LabelReferences<'_> {
        fn visit_break_statement(&mut self, statement: &oxc_ast::ast::BreakStatement<'a>) {
            if statement.label.as_ref().is_some_and(|label| label.name == self.name) {
                self.found = true;
            }
        }

        fn visit_continue_statement(&mut self, statement: &oxc_ast::ast::ContinueStatement<'a>) {
            if statement.label.as_ref().is_some_and(|label| label.name == self.name) {
                self.found = true;
            }
        }

        fn visit_labeled_statement(&mut self, labeled: &oxc_ast::ast::LabeledStatement<'a>) {
            if labeled.label.name != self.name {
                oxc_ast_visit::walk::walk_labeled_statement(self, labeled);
            }
        }

        fn visit_function(&mut self, _: &oxc_ast::ast::Function<'a>, _: oxc_syntax::scope::ScopeFlags) {}

        fn visit_arrow_function_expression(&mut self, _: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}

        fn visit_static_block(&mut self, _: &oxc_ast::ast::StaticBlock<'a>) {}
    }

    let mut references = LabelReferences { name, found: false };
    references.visit_statement(body);
    references.found
}

/// The type references in a type parameter's default that resolve to one of
/// the parameters from `later` (the parameter itself and those after it).
struct LaterTypeParameterReferences<'n> {
    later: Vec<&'n str>,
    /// Names redeclared by an enclosing nested scope, innermost last.
    shadowed: Vec<&'n str>,
    found: Vec<Span>,
}

impl<'a> LaterTypeParameterReferences<'a> {
    fn scoped(&mut self, walk: impl FnOnce(&mut Self)) {
        let depth = self.shadowed.len();
        walk(self);
        self.shadowed.truncate(depth);
    }
}

impl<'a> Visit<'a> for LaterTypeParameterReferences<'a> {
    fn visit_ts_type_reference(&mut self, reference: &oxc_ast::ast::TSTypeReference<'a>) {
        if let oxc_ast::ast::TSTypeName::IdentifierReference(identifier) = &reference.type_name
            && self.later.contains(&identifier.name.as_str())
            && !self.shadowed.contains(&identifier.name.as_str())
        {
            self.found.push(reference.span);
        }
        oxc_ast_visit::walk::walk_ts_type_reference(self, reference);
    }

    // A signature's own parameters and an `infer` capture arrive here; the
    // enclosing node bounds the scope.
    fn visit_ts_type_parameter(&mut self, parameter: &oxc_ast::ast::TSTypeParameter<'a>) {
        self.shadowed.push(parameter.name.name.as_str());
        oxc_ast_visit::walk::walk_ts_type_parameter(self, parameter);
    }

    fn visit_ts_function_type(&mut self, function: &oxc_ast::ast::TSFunctionType<'a>) {
        self.scoped(|this| oxc_ast_visit::walk::walk_ts_function_type(this, function));
    }

    fn visit_ts_constructor_type(&mut self, constructor: &oxc_ast::ast::TSConstructorType<'a>) {
        self.scoped(|this| oxc_ast_visit::walk::walk_ts_constructor_type(this, constructor));
    }

    fn visit_ts_method_signature(&mut self, method: &oxc_ast::ast::TSMethodSignature<'a>) {
        self.scoped(|this| oxc_ast_visit::walk::walk_ts_method_signature(this, method));
    }

    fn visit_ts_call_signature_declaration(
        &mut self,
        signature: &oxc_ast::ast::TSCallSignatureDeclaration<'a>,
    ) {
        self.scoped(|this| oxc_ast_visit::walk::walk_ts_call_signature_declaration(this, signature));
    }

    fn visit_ts_construct_signature_declaration(
        &mut self,
        signature: &oxc_ast::ast::TSConstructSignatureDeclaration<'a>,
    ) {
        self.scoped(|this| {
            oxc_ast_visit::walk::walk_ts_construct_signature_declaration(this, signature);
        });
    }

    // oxc keeps a mapped type's key as a plain binding rather than a type
    // parameter.
    fn visit_ts_mapped_type(&mut self, mapped: &oxc_ast::ast::TSMappedType<'a>) {
        self.scoped(|this| {
            this.shadowed.push(mapped.key.name.as_str());
            oxc_ast_visit::walk::walk_ts_mapped_type(this, mapped);
        });
    }

    // An `infer` capture is in scope for the `extends` clause and the true
    // branch only.
    fn visit_ts_conditional_type(&mut self, conditional: &oxc_ast::ast::TSConditionalType<'a>) {
        self.scoped(|this| {
            this.visit_ts_type(&conditional.check_type);
            this.visit_ts_type(&conditional.extends_type);
            this.visit_ts_type(&conditional.true_type);
        });
        self.visit_ts_type(&conditional.false_type);
    }
}

impl<'a> Visit<'a> for ContextCollector<'a, '_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        if self.index_signatures_only {
            if let AstKind::TSIndexSignature(signature) = kind {
                self.check_index_signature(signature);
            }
            self.stack.push(kind);
            return;
        }
        self.check_declaration_position(&kind);
        self.check_member_placement(&kind);
        self.check_commonjs_module_syntax(&kind);
        match kind {
            AstKind::BreakStatement(statement) => {
                self.check_jump(statement.span, statement.label.as_ref().map(|l| l.name.as_str()), false);
            }
            AstKind::ContinueStatement(statement) => {
                self.check_jump(statement.span, statement.label.as_ref().map(|l| l.name.as_str()), true);
            }
            AstKind::LabeledStatement(statement) => self.check_labeled_statement(statement),
            AstKind::FormalParameter(parameter) => {
                self.check_parameter_property(parameter);
                if parameter.accessibility.is_some() || parameter.readonly || parameter.r#override {
                    self.push_erasable(parameter.span);
                }
            }
            AstKind::TSTypeAssertion(assertion) => {
                self.push_erasable(Span::new(assertion.span.start, assertion.expression.span().start));
                self.check_const_assertion(&assertion.type_annotation, &assertion.expression);
            }
            AstKind::TSAsExpression(assertion) => {
                self.check_const_assertion(&assertion.type_annotation, &assertion.expression);
            }
            AstKind::WithStatement(statement) => {
                self.check_with_statement(statement);
                self.with_bodies.push(statement.body.span());
            }
            AstKind::BindingRestElement(rest) => self.check_rest_binding_property_name(rest),
            AstKind::BindingIdentifier(identifier) => {
                self.check_contextual_identifier(&identifier.name, identifier.span);
                if self.is_strict_checked_binding() {
                    self.check_eval_or_arguments(&identifier.name, identifier.span);
                }
            }
            AstKind::IdentifierReference(identifier) => {
                self.check_contextual_identifier(&identifier.name, identifier.span);
                self.check_const_enum_reference(identifier);
            }
            AstKind::ComputedMemberExpression(member) => self.check_const_enum_element_access(member),
            AstKind::LabelIdentifier(identifier) => {
                self.check_contextual_identifier(&identifier.name, identifier.span);
            }
            AstKind::AssignmentExpression(assignment) => {
                if let AssignmentTarget::AssignmentTargetIdentifier(identifier) = &assignment.left {
                    self.check_eval_or_arguments(&identifier.name, identifier.span);
                }
                self.check_private_method_assignment(&assignment.left);
            }
            AstKind::UpdateExpression(update) => {
                if let SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) = &update.argument {
                    self.check_eval_or_arguments(&identifier.name, identifier.span);
                }
                self.check_private_method_update(&update.argument);
            }
            AstKind::PrivateFieldExpression(access) => {
                self.check_private_setter_read(access);
                // tsc's `checkPropertyAccessExpressionOrQualifiedName`: an
                // undeclared private name inside a class (TS1111), kept only
                // for plain JavaScript.
                if self.javascript
                    && self.stack.iter().any(|kind| matches!(kind, AstKind::Class(_)))
                    && !self.declares_private_name(&access.field.name)
                {
                    let name = format!("#{}", access.field.name);
                    self.out.push(ParsedGrammarDiagnostic {
                        kind: Kind::Ts(1111),
                        span: text_span_from_oxc_span(access.field.span),
                        name: Some(name),
                    });
                }
            }
            // A comma expression in a JSX expression container (TS18007).
            AstKind::JSXExpressionContainer(container) => {
                if let oxc_ast::ast::JSXExpression::SequenceExpression(sequence) = &container.expression {
                    self.push(18007, sequence.span, &[]);
                }
            }
            // Outside a module or ambient module, `declare global` is TS2669
            // and contributes nothing to the globals.
            AstKind::TSGlobalDeclaration(global)
                if self.external_module
                    || self.stack.iter().any(|kind| matches!(kind, AstKind::TSModuleDeclaration(_))) =>
            {
                self.check_global_augmentation_names(&global.body.body);
            }
            AstKind::MetaProperty(meta) => {
                self.check_meta_property_name(meta);
                self.check_new_target(meta);
                // `checkImportMetaProperty` under a node module kind; the
                // checker keeps it for a file that is not ESM.
                if meta.meta.name == "import" && meta.property.name == "meta" {
                    self.push(1470, meta.span, &[]);
                }
            }
            // checker.go `checkTypePredicate` never types a `this` predicate's
            // name, so it is no `this` type to check.
            AstKind::TSThisType(this_type)
                if !matches!(self.stack.last(), Some(AstKind::TSTypePredicate(predicate))
                    if matches!(&predicate.parameter_name, oxc_ast::ast::TSTypePredicateName::This(name) if name.span == this_type.span)) =>
            {
                self.check_this_type(this_type.span)
            }
            AstKind::TSInferType(infer) => {
                self.check_infer_type(infer.span);
                self.record_infer_declaration(infer);
            }
            AstKind::TSInterfaceDeclaration(declaration) => {
                self.check_reserved_type_name(2427, &declaration.id);
                self.check_circular_type_parameter_defaults(&declaration.id.name, declaration.type_parameters.as_deref());
            }
            AstKind::TSTypeAliasDeclaration(declaration) => {
                self.check_reserved_type_name(2457, &declaration.id);
                self.check_circular_type_parameter_defaults(&declaration.id.name, declaration.type_parameters.as_deref());
                if let oxc_ast::ast::TSType::TSIntrinsicKeyword(keyword) = &declaration.type_annotation {
                    let parameter_count =
                        declaration.type_parameters.as_ref().map_or(0, |parameters| parameters.params.len());
                    let name = declaration.id.name.as_str();
                    let valid = (parameter_count == 0 && name == "BuiltinIteratorReturn")
                        || (parameter_count == 1
                            && matches!(name, "Uppercase" | "Lowercase" | "Capitalize" | "Uncapitalize" | "NoInfer"));
                    if !valid {
                        self.push(2795, keyword.span, &[]);
                    }
                }
            }
            AstKind::TSEnumDeclaration(declaration) => {
                self.check_reserved_type_name(2431, &declaration.id);
                if self.ambient_depth == 0 && !declaration.declare {
                    self.push_erasable(declaration.id.span);
                }
            }
            AstKind::TSEnumMember(member) => {
                self.check_enum_member_overshift(member);
                self.check_enum_member_numeric_name(member);
            }
            AstKind::Class(class) => {
                check_class_name(self, class);
                self.check_object_class_name(class);
                if let Some(id) = &class.id {
                    self.check_circular_type_parameter_defaults(&id.name, class.type_parameters.as_deref());
                }
            }
            AstKind::CatchClause(clause) => self.check_catch_clause(clause),
            AstKind::VariableDeclarator(declarator) => {
                self.check_let_name(declarator);
                self.check_definite_assertion(declarator);
                self.check_var_declared_name_not_shadowed(declarator);
            }
            AstKind::TSModuleDeclaration(module) => {
                self.check_relative_ambient_module_name(module);
                if self.ambient_depth == 0
                    && !module.declare
                    && let oxc_ast::ast::TSModuleDeclarationName::Identifier(name) = &module.id
                    && module_instance_state(module) == ModuleInstanceState::Instantiated
                {
                    self.push_erasable(name.span);
                    // `checkModuleDeclaration` under `isolatedModules`, which
                    // the checker decides along with the file's module-ness.
                    if !self.external_module && matches!(self.stack.last(), Some(AstKind::Program(_))) {
                        self.push(1280, name.span, &[]);
                    }
                }
            }
            AstKind::PropertyDefinition(property) => {
                // `checkGrammarProperty`'s `!` rules for a class property.
                if property.definite {
                    let code = if property.value.is_some() {
                        Some(1263)
                    } else if property.type_annotation.is_none() {
                        Some(1264)
                    } else if property.r#static
                        || property.declare
                        || self.ambient_depth > 0
                        || property.r#type == oxc_ast::ast::PropertyDefinitionType::TSAbstractPropertyDefinition
                    {
                        Some(1255)
                    } else {
                        None
                    };
                    if let Some(code) = code {
                        self.push_at_exclamation(code, property.key.span());
                    }
                }
                // tsc checks the name only once `checkGrammarModifiers` passes,
                // so not under a decorator TS1206 rejects; a `[k in T]` name is
                // a misplaced mapped type instead (TS7061).
                if property.computed
                    && !self.check_mapped_type_member(&property.key)
                    && let Some(span) = self.dynamic_property_name_span(&property.key)
                {
                    let kind = if property.decorators.is_empty() {
                        Some(Kind::Ts(1166))
                    } else {
                        self.decorator_legality(kind, self.stack.len())
                            .and_then(|(_, legality)| legality.if_legal(1166))
                    };
                    if let Some(kind) = kind {
                        self.out.push(ParsedGrammarDiagnostic {
                            kind,
                            span: text_span_from_oxc_span(span),
                            name: None,
                        });
                    }
                }
            }
            AstKind::AccessorProperty(property) if property.computed => {
                if !self.check_mapped_type_member(&property.key) && property.decorators.is_empty() {
                    self.check_dynamic_property_name(1166, &property.key);
                }
            }
            AstKind::TSMethodSignature(signature) if signature.computed => {
                let code = match self.stack.last() {
                    Some(AstKind::TSInterfaceBody(_)) => 1169,
                    _ => 1170,
                };
                self.check_dynamic_property_name(code, &signature.key);
            }
            AstKind::TSPropertySignature(signature) if signature.computed => {
                if !self.check_mapped_type_member(&signature.key) {
                    let code = match self.stack.last() {
                        Some(AstKind::TSInterfaceBody(_)) => 1169,
                        _ => 1170,
                    };
                    self.check_dynamic_property_name(code, &signature.key);
                }
            }
            AstKind::VariableDeclaration(declaration) => {
                self.check_es_module_marker(declaration);
                self.check_single_statement_declaration(declaration);
                if declaration.declare && !self.at_module_element_level() {
                    let start = declaration.span.start;
                    self.push(1184, Span::new(start, start + 7), &[]);
                }
                self.check_using_declaration_list(declaration);
            }
            AstKind::YieldExpression(expression) => self.check_yield(expression),
            AstKind::AwaitExpression(expression) => self.check_await(expression),
            AstKind::ForOfStatement(statement) => {
                if statement.r#await {
                    self.check_for_await(statement);
                } else {
                    self.check_for_of_async_target(statement);
                }
                self.check_for_in_or_of_declaration(&statement.left, false);
                self.check_private_method_for_target(&statement.left);
            }
            AstKind::ForInStatement(statement) => {
                self.check_for_in_or_of_declaration(&statement.left, true);
                self.check_private_method_for_target(&statement.left);
            }
            AstKind::FormalParameterRest(rest) => self.check_rest_parameter_type(rest),
            // `default` is a keyword no declaration binds as a value name, so a
            // query of it never resolves, whatever the file default-exports.
            AstKind::TSTypeQuery(query) => {
                if let oxc_ast::ast::TSTypeQueryExprName::IdentifierReference(identifier) = &query.expr_name
                    && identifier.name == "default"
                {
                    self.push(2304, identifier.span, &["default"]);
                }
            }
            AstKind::BindingProperty(property) => self.check_renamed_binding_in_signature(property),
            AstKind::FormalParameters(parameters) => self.check_parameter_initializer_references(parameters),
            AstKind::Super(expression) => self.check_super(expression.span),
            AstKind::ThisExpression(expression) => self.check_this_expression(expression.span),
            AstKind::TSIndexSignature(signature) => self.check_index_signature(signature),
            AstKind::TSTypeOperator(operator) => self.check_unique_symbol_position(operator),
            // oxc keeps a static block's span from its first modifier; any
            // text before `static` is one — TS1184 (oxc reports it too, and a
            // grammar TS1184 elsewhere in the file claims oxc's copies), or
            // TS1206 when it is a decorator (`checkGrammarModifiers` meets it
            // first).
            AstKind::StaticBlock(block) => {
                let text = &self.source_text[block.span.start as usize..block.span.end as usize];
                if let Some(keyword) = text.find("static")
                    && !text[..keyword].trim().is_empty()
                {
                    let start = block.span.start;
                    if text.starts_with('@') {
                        self.push(1206, Span::new(start, start + 1), &[]);
                    } else {
                        let end = first_token_end(self.source_text, start as usize) as u32;
                        self.push(1184, Span::new(start, end), &[]);
                    }
                }
            }
            AstKind::MethodDefinition(method) => {
                self.check_accessor_body(method);
                self.check_generator_constructor(method);
            }
            AstKind::TSTypeParameterDeclaration(declaration) => {
                self.check_circular_constraints(declaration);
                self.check_type_parameter_defaults(declaration);
            }
            AstKind::TSMappedType(mapped) => self.check_circular_mapped_key(mapped),
            AstKind::Decorator(decorator) => self.check_decorator_target(decorator),
            AstKind::JSDocNonNullableType(node) => {
                self.check_jsdoc_nullability("!", node.postfix, node.span, &node.type_annotation);
            }
            AstKind::JSDocNullableType(node) => {
                self.check_jsdoc_nullability("?", node.postfix, node.span, &node.type_annotation);
            }
            AstKind::TSImportEqualsDeclaration(declaration) => {
                // tsc's `checkGrammarModuleElementContext` bails first.
                if !self.at_module_element_level() {
                    let span = self.first_token(self.statement_start(declaration.span.start));
                    self.push(self.misplaced_import_code(), span, &[]);
                } else {
                    if self.ambient_depth == 0 {
                        let start = self.statement_start(declaration.span.start);
                        self.push_erasable(Span::new(start, declaration.span.end));
                    }
                    // An accessibility or `static` modifier is TS1044 on any
                    // module element, whatever its kind.
                    self.check_statement_modifiers(NodeKind::InterfaceDeclaration, declaration.span);
                    let require_in_namespace = self.check_import_require_in_namespace(declaration);
                    self.check_import_alias_name(declaration);
                    // Gated on `module` by the checker: ES2015..ESNext cannot emit it.
                    // Inside a namespace it is TS1147 instead, and tsc stops there
                    // (`checkExternalImportOrExportDeclaration`).
                    if !require_in_namespace
                        && !matches!(self.stack.last(), Some(AstKind::TSModuleBlock(_)))
                        && matches!(
                            declaration.module_reference,
                            oxc_ast::ast::TSModuleReference::ExternalModuleReference(_)
                        )
                        && declaration.import_kind.is_value()
                        && self.ambient_depth == 0
                    {
                        self.push(1202, declaration.span, &[]);
                    }
                }
            }
            AstKind::TSExportAssignment(assignment) => {
                if self.ambient_depth == 0 && self.at_module_element_level() {
                    self.push_erasable(assignment.span);
                }
                if !self.at_module_element_level() {
                    self.push(1231, self.first_token(assignment.span.start), &[]);
                } else if self.is_contained_by_namespace() {
                    // tsc reports TS1063 here and stops; that code is not emitted.
                } else if self.ambient_depth == 0 {
                    // Gated on `module` and the file's emit format by the checker.
                    self.push(1203, assignment.span, &[]);
                }
            }
            AstKind::TSTypeParameter(parameter) => {
                self.check_reserved_type_name(2368, &parameter.name);
                self.check_variance_modifier_owner(parameter);
            }
            AstKind::ImportDeclaration(declaration) => {
                if !self.at_module_element_level() {
                    self.push(self.misplaced_import_code(), self.first_token(declaration.span.start), &[]);
                } else if let Some(clause) = &declaration.with_clause {
                    self.check_import_assertion(clause);
                    self.check_import_attributes_supported(clause, declaration.import_kind.is_type());
                }
            }
            AstKind::ExportNamedDeclaration(declaration) => {
                if !self.at_module_element_level() {
                    // A wrapped declaration's `export` is a misplaced modifier;
                    // an export list is a misplaced export declaration. A
                    // namespace, global augmentation or import alias reports
                    // its own context error on this `export` instead.
                    let code = match &declaration.declaration {
                        Some(
                            oxc_ast::ast::Declaration::TSModuleDeclaration(_)
                            | oxc_ast::ast::Declaration::TSGlobalDeclaration(_)
                            | oxc_ast::ast::Declaration::TSImportEqualsDeclaration(_),
                        ) => None,
                        Some(_) => Some(1184),
                        None => Some(self.misplaced_export_code()),
                    };
                    if let Some(code) = code {
                        let start = declaration.span.start;
                        self.push(code, Span::new(start, start + 6), &[]);
                    }
                } else if let Some(clause) = &declaration.with_clause {
                    self.check_import_assertion(clause);
                    self.check_import_attributes_supported(clause, declaration.export_kind.is_type());
                }
            }
            AstKind::ExportAllDeclaration(declaration) => {
                if !self.at_module_element_level() {
                    let start = declaration.span.start;
                    self.push(self.misplaced_export_code(), Span::new(start, start + 6), &[]);
                } else if let Some(clause) = &declaration.with_clause {
                    self.check_import_assertion(clause);
                    self.check_import_attributes_supported(clause, declaration.export_kind.is_type());
                }
            }
            AstKind::ClassBody(body) => {
                self.check_private_name_staticness(body);
                self.check_static_function_property_names(body);
                self.check_duplicate_index_signatures(body.body.iter().filter_map(|element| {
                    match element {
                        oxc_ast::ast::ClassElement::TSIndexSignature(signature) => Some(&**signature),
                        _ => None,
                    }
                }));
            }
            AstKind::TSInterfaceBody(body) => {
                self.check_duplicate_index_signatures(signature_index_signatures(&body.body));
                self.check_signature_overload_optionality(&body.body);
            }
            AstKind::TSTypeLiteral(literal) => {
                self.check_duplicate_index_signatures(signature_index_signatures(&literal.members));
                self.check_signature_overload_optionality(&literal.members);
            }
            AstKind::TSModuleBlock(block) => {
                self.check_statements_in_ambient_context(&block.body);
                self.check_redundant_declare(&block.body);
                self.check_enum_merges(&block.body);
                self.check_self_referencing_annotations(&block.body);
                self.check_type_parameter_lists_identical(&block.body);
                self.check_merged_declarations(&block.body);
            }
            AstKind::BlockStatement(block) => {
                self.check_statements_in_ambient_context(&block.body);
                self.check_enum_merges(&block.body);
                self.check_self_referencing_annotations(&block.body);
                self.check_merged_declarations(&block.body);
            }
            AstKind::Program(program) => {
                self.check_enum_merges(&program.body);
                self.check_self_referencing_annotations(&program.body);
                self.check_type_parameter_lists_identical(&program.body);
                if self.external_module {
                    self.check_export_assignment_conflicts(program);
                }
                self.check_ambient_module_export_assignments(program);
                self.check_top_level_names(program);
                self.check_reflect_collisions(program);
                self.check_merged_declarations(&program.body);
                super::grammar_merges::check_namespace_enum_initializers(&program.body, self.out);
            }
            AstKind::FunctionBody(body) => {
                self.check_implementation_in_ambient_context(body);
                self.check_enum_merges(&body.statements);
                self.check_self_referencing_annotations(&body.statements);
                self.check_merged_declarations(&body.statements);
            }
            _ => {}
        }
        if is_ambient_marker(&kind) {
            self.ambient_depth += 1;
        }
        self.stack.push(kind);
    }

    fn leave_node(&mut self, kind: AstKind<'a>) {
        self.stack.pop();
        if self.index_signatures_only {
            return;
        }
        if let AstKind::TSConditionalType(conditional) = &kind {
            self.check_infer_constraints(conditional.span.start);
        }
        if is_ambient_marker(&kind) {
            self.ambient_depth -= 1;
        }
    }

    // An export specifier's local name is an identifier name to tsc, and so
    // is the name on either side of `as`.
    fn visit_export_specifier(&mut self, _: &oxc_ast::ast::ExportSpecifier<'a>) {}

    fn visit_module_export_name(&mut self, name: &ModuleExportName<'a>) {
        if !matches!(name, ModuleExportName::IdentifierReference(_)) {
            oxc_ast_visit::walk::walk_module_export_name(self, name);
        }
    }
}

/// The modifier codes this pass reports from tsc's `checkGrammarModifiers`
/// port; the rest of its table is oxc's or another check's to report.
const OWNED_MODIFIER_CODES: &[u32] = &[1024, 1029, 1031, 1040, 1042, 1044, 1089, 1243, 1274, 1277, 1319, 18019];

/// Declaration-position and modifier grammar: tsc's
/// `checkGrammarModuleElementContext`, `checkModuleDeclaration`,
/// `checkGrammarModifiers`, `checkModuleAugmentationElement`, and the
/// `super`/`instanceof` type-argument rules.
impl<'a> ContextCollector<'a, '_> {
    fn check_declaration_position(&mut self, kind: &AstKind<'a>) {
        match kind {
            AstKind::Function(function) => {
                if matches!(
                    function.r#type,
                    oxc_ast::ast::FunctionType::FunctionDeclaration
                        | oxc_ast::ast::FunctionType::TSDeclareFunction
                ) {
                    self.check_statement_modifiers(NodeKind::FunctionDeclaration, function.span);
                }
                if let Some(body) = &function.body {
                    self.check_use_strict_parameters(&function.params, body);
                }
            }
            AstKind::ArrowFunctionExpression(arrow) if !arrow.expression => {
                self.check_use_strict_parameters(&arrow.params, &arrow.body);
            }
            AstKind::Class(class) if class.r#type == oxc_ast::ast::ClassType::ClassDeclaration => {
                self.check_statement_modifiers_after(NodeKind::ClassDeclaration, class.span, &class.decorators);
            }
            AstKind::VariableDeclaration(declaration) => {
                let parent = self.stack.iter().rev().find(|kind| {
                    !matches!(kind, AstKind::ExportNamedDeclaration(_))
                });
                if !matches!(
                    parent,
                    Some(
                        AstKind::ForStatement(_)
                            | AstKind::ForInStatement(_)
                            | AstKind::ForOfStatement(_)
                    )
                ) {
                    self.check_statement_modifiers(NodeKind::VariableStatement, declaration.span);
                }
            }
            AstKind::TSInterfaceDeclaration(declaration) => {
                self.check_statement_modifiers(NodeKind::InterfaceDeclaration, declaration.span);
            }
            AstKind::TSTypeAliasDeclaration(declaration) => {
                self.check_statement_modifiers(NodeKind::TypeAliasDeclaration, declaration.span);
            }
            AstKind::TSEnumDeclaration(declaration) => {
                self.check_statement_modifiers(NodeKind::EnumDeclaration, declaration.span);
            }
            AstKind::TSModuleDeclaration(declaration) => self.check_module_declaration(declaration),
            AstKind::TSGlobalDeclaration(declaration) => self.check_global_declaration(declaration),
            AstKind::ExportDefaultDeclaration(declaration) => self.check_export_default(declaration),
            AstKind::PropertyDefinition(property) => {
                let start = decorators_end(&property.decorators, property.span.start);
                self.check_class_member_modifiers(
                    NodeKind::PropertyDeclaration,
                    start,
                    &property.key,
                );
                // A JSDoc `@type {unique symbol}` (which the lowering reads as
                // `symbol`) needs `static` and `@readonly` like the written
                // type does (TS1331).
                if self.javascript
                    && property.type_annotation.is_none()
                    && let Some((_, type_span)) = super::jsdoc::declared_type_at(property.span.start)
                    && self
                        .source_text
                        .get(type_span.start..type_span.end)
                        .is_some_and(|text| text.split_whitespace().eq(["unique", "symbol"]))
                    && !(property.r#static
                        && (property.readonly
                            || super::jsdoc::member_has_modifier(
                                property.span.start,
                                super::jsdoc::JsDocModifier::Readonly,
                            )))
                {
                    self.push(1331, property.key.span(), &[]);
                }
            }
            AstKind::AccessorProperty(property) => {
                let start = decorators_end(&property.decorators, property.span.start);
                self.check_class_member_modifiers(
                    NodeKind::PropertyDeclaration,
                    start,
                    &property.key,
                );
            }
            AstKind::MethodDefinition(method) => {
                // tsc parses any method named `constructor` as a constructor
                // (`tryParseConstructorDeclaration`); oxc makes `static
                // constructor()` a static method.
                let named_constructor = !method.computed
                    && match &method.key {
                        oxc_ast::ast::PropertyKey::StaticIdentifier(key) => key.name == "constructor",
                        oxc_ast::ast::PropertyKey::StringLiteral(key) => key.value == "constructor",
                        _ => false,
                    };
                let node = match method.kind {
                    MethodDefinitionKind::Constructor => NodeKind::Constructor,
                    MethodDefinitionKind::Method if named_constructor => NodeKind::Constructor,
                    MethodDefinitionKind::Method => NodeKind::MethodDeclaration,
                    MethodDefinitionKind::Get => NodeKind::GetAccessor,
                    MethodDefinitionKind::Set => NodeKind::SetAccessor,
                };
                let start = decorators_end(&method.decorators, method.span.start);
                self.check_class_member_modifiers(node, start, &method.key);
            }
            AstKind::ObjectProperty(property) => {
                self.check_object_member_modifiers(property);
                // `checkGrammarObjectLiteralExpression`: a shorthand's
                // `= initializer` outside a destructuring pattern, which the
                // parser drops but leaves inside the property's span (TS1312
                // on the `=`).
                if property.shorthand && property.span.end > property.key.span().end {
                    let rest = &self.source_text[property.key.span().end as usize..property.span.end as usize];
                    if let Some(offset) = rest.find('=') {
                        let start = property.key.span().end + offset as u32;
                        self.push(1312, Span::new(start, start + 1), &[]);
                    }
                }
            }
            AstKind::TSTypeParameter(parameter) => self.check_type_parameter_modifiers(parameter),
            AstKind::FormalParameter(parameter) => {
                self.check_parameter_modifiers(parameter);
                self.check_demoted_rest_parameter(parameter);
            }
            AstKind::TSTypeParameterInstantiation(instantiation) => {
                self.check_type_argument_trailing_comma(instantiation);
            }
            AstKind::TSInstantiationExpression(expression) => {
                if matches!(expression.expression, oxc_ast::ast::Expression::Super(_)) {
                    self.push(2754, expression.type_arguments.span, &[]);
                }
            }
            AstKind::TaggedTemplateExpression(tagged) => {
                if let Some(type_arguments) = &tagged.type_arguments
                    && matches!(tagged.tag, oxc_ast::ast::Expression::Super(_))
                {
                    self.push(2754, type_arguments.span, &[]);
                }
            }
            AstKind::CallExpression(call) => {
                if let Some(type_arguments) = &call.type_arguments
                    && matches!(call.callee, oxc_ast::ast::Expression::Super(_))
                {
                    self.push(2754, type_arguments.span, &[]);
                }
            }
            // `checkGrammarImportCallExpression` rejects a dynamic import
            // under `module: es2015`, and a second argument under a module
            // kind that takes none; the checker gates both on the option. The
            // parser leaves an empty-span placeholder for a spread argument.
            AstKind::ImportExpression(import) if import.phase.is_none() => {
                self.push(1323, import.span, &[]);
                if let Some(options) = &import.options
                    && !options.span().is_empty()
                {
                    self.push(1324, options.span(), &[]);
                }
                // `checkGrammarForDisallowedTrailingComma`, marked for the same
                // gating.
                let bytes = self.source_text.as_bytes();
                let last = import.options.as_ref().unwrap_or(&import.source).span();
                let comma = members::skip_trivia(self.source_text, last.end as usize);
                if comma < import.span.end as usize
                    && bytes[comma] == b','
                    && bytes.get(members::skip_trivia(self.source_text, comma + 1)) == Some(&b')')
                {
                    self.push(1009, Span::new(comma as u32, comma as u32 + 1), &["import"]);
                }
            }
            AstKind::BinaryExpression(binary) => self.check_instanceof_instantiation(binary),
            _ => {}
        }
    }

    /// The ancestors as tsc sees them: an `export` wrapper is a modifier of
    /// the declaration it wraps, not its parent.
    fn ancestors_as_tsc(&self) -> impl Iterator<Item = AstKind<'a>> + '_ {
        self.stack
            .iter()
            .rev()
            .filter(|kind| !matches!(kind, AstKind::ExportNamedDeclaration(_)))
            .copied()
    }

    /// The span tsc's `grammarErrorOnFirstToken` reports at.
    fn first_token(&self, start: u32) -> Span {
        Span::new(start, first_token_end(self.source_text, start as usize) as u32)
    }

    fn push_modifier_error(&mut self, error: Option<modifiers::ModifierError>) {
        // oxc reports a misplaced `abstract` only outside an ambient context.
        if let Some(error) = error
            && (OWNED_MODIFIER_CODES.contains(&error.code) || (error.code == 1242 && self.ambient_depth > 0))
        {
            self.push(error.code, error.span, &error.args);
        }
    }

    /// Modifiers on a statement-level declaration, read from the statement's
    /// start (an `export` wrapper's, when there is one) up to its keyword.
    fn check_statement_modifiers(&mut self, node: NodeKind, span: Span) {
        self.check_statement_modifiers_after(node, span, &[]);
    }

    /// [`Self::check_statement_modifiers`] for a declaration whose leading
    /// decorators come before its modifiers (`@d default class {}`).
    fn check_statement_modifiers_after(
        &mut self,
        node: NodeKind,
        span: Span,
        decorators: &[oxc_ast::ast::Decorator<'_>],
    ) {
        let mut ancestors = self.stack.iter().rev();
        let mut parent = ancestors.next();
        let mut start = span.start;
        match parent {
            Some(AstKind::ExportNamedDeclaration(export)) => {
                start = export.span.start;
                parent = ancestors.next();
            }
            Some(AstKind::ExportDefaultDeclaration(export)) => {
                start = export.span.start;
                parent = ancestors.next();
            }
            _ => {}
        }
        let parent = match parent {
            None | Some(AstKind::Program(_)) => Parent::SourceFile,
            Some(AstKind::TSModuleBlock(_)) => Parent::ModuleBlock {
                namespace: matches!(
                    ancestors.next(),
                    Some(AstKind::TSModuleDeclaration(module))
                        if matches!(module.id, oxc_ast::ast::TSModuleDeclarationName::Identifier(_))
                ),
            },
            _ => Parent::Other,
        };
        let Some(modifiers) = modifiers::scan_modifiers(
            self.source_text,
            decorators_end(decorators, start),
            span.end,
            node != NodeKind::VariableStatement,
        ) else {
            return;
        };
        let context = ModifierContext {
            node,
            parent,
            parent_ambient: self.ambient_depth > 0,
            name_is_private: false,
            type_parameter_owner: TypeParameterOwner::Other,
        };
        if node == NodeKind::FunctionDeclaration
            && matches!(context.parent, Parent::Other)
            && start == span.start
        {
            // oxc accepts `declare function` in a block without a word; an
            // `export` wrapper reports its own TS1184.
            self.examined_modifiers.extend(modifiers.iter().map(|modifier| modifier.span.start));
            if let Some(error) = modifiers::first_modifier_error(&modifiers, &context)
                && (error.code == 1184 || OWNED_MODIFIER_CODES.contains(&error.code))
            {
                self.push(error.code, error.span, &error.args);
            }
            return;
        }
        self.examine_modifiers(&modifiers, &context);
    }

    fn examine_modifiers(&mut self, modifiers: &[modifiers::Modifier], context: &ModifierContext) {
        self.examined_modifiers.extend(modifiers.iter().map(|modifier| modifier.span.start));
        self.push_modifier_error(modifiers::first_modifier_error(modifiers, context));
    }

    /// A parameter property's modifiers (`checkParameter`); oxc keeps which
    /// ones it has, not the order they were written in.
    fn check_parameter_modifiers(&mut self, parameter: &oxc_ast::ast::FormalParameter<'_>) {
        if parameter.accessibility.is_none() && !parameter.readonly && !parameter.r#override {
            return;
        }
        let start = decorators_end(&parameter.decorators, parameter.span.start);
        let Some(modifiers) =
            modifiers::scan_modifiers(self.source_text, start, parameter.pattern.span().start, true)
        else {
            return;
        };
        let context = ModifierContext {
            node: NodeKind::Parameter,
            parent: Parent::Other,
            parent_ambient: self.ambient_depth > 0,
            name_is_private: false,
            type_parameter_owner: TypeParameterOwner::Other,
        };
        self.examine_modifiers(&modifiers, &context);
    }

    fn check_class_member_modifiers(
        &mut self,
        node: NodeKind,
        start: u32,
        key: &oxc_ast::ast::PropertyKey<'_>,
    ) {
        let Some(AstKind::Class(class)) = self.stack.iter().rev().nth(1) else {
            return;
        };
        let Some(mut modifiers) =
            modifiers::scan_modifiers(self.source_text, start, key.span().start, true)
        else {
            return;
        };
        // A JSDoc modifier follows the written ones (`reparseHosted`) and is
        // exempt from their order. oxc never sees it, so an error on it is
        // this walker's to report.
        let jsdoc_modifiers = super::jsdoc::member_modifiers_at(start);
        let jsdoc_starts: Vec<u32> = jsdoc_modifiers.iter().map(|(_, span)| span.start as u32).collect();
        modifiers.extend(jsdoc_modifiers.into_iter().map(|(modifier, span)| modifiers::Modifier {
            kind: match modifier {
                super::jsdoc::JsDocModifier::Public => modifiers::ModifierKind::Public,
                super::jsdoc::JsDocModifier::Private => modifiers::ModifierKind::Private,
                super::jsdoc::JsDocModifier::Protected => modifiers::ModifierKind::Protected,
                super::jsdoc::JsDocModifier::Readonly => modifiers::ModifierKind::Readonly,
                super::jsdoc::JsDocModifier::Override => modifiers::ModifierKind::Override,
            },
            span: oxc_span::Span::new(span.start as u32, span.end as u32),
            reparsed: true,
        }));
        let context = ModifierContext {
            node,
            parent: Parent::Class {
                is_declaration: class.r#type == oxc_ast::ast::ClassType::ClassDeclaration,
                is_abstract: class.r#abstract,
            },
            parent_ambient: self.ambient_depth > 0,
            name_is_private: matches!(key, oxc_ast::ast::PropertyKey::PrivateIdentifier(_)),
            type_parameter_owner: TypeParameterOwner::Other,
        };
        self.examined_modifiers.extend(modifiers.iter().map(|modifier| modifier.span.start));
        match modifiers::first_modifier_error(&modifiers, &context) {
            Some(error) if jsdoc_starts.contains(&error.span.start) => {
                self.push(error.code, error.span, &error.args);
            }
            error => self.push_modifier_error(error),
        }
    }

    fn check_type_parameter_modifiers(&mut self, parameter: &oxc_ast::ast::TSTypeParameter<'_>) {
        let Some(modifiers) = modifiers::scan_modifiers(
            self.source_text,
            parameter.span.start,
            parameter.name.span.start,
            true,
        ) else {
            return;
        };
        let owner = match self.stack.iter().rev().nth(1) {
            Some(
                AstKind::Function(_)
                | AstKind::ArrowFunctionExpression(_)
                | AstKind::TSFunctionType(_)
                | AstKind::TSConstructorType(_)
                | AstKind::TSCallSignatureDeclaration(_)
                | AstKind::TSConstructSignatureDeclaration(_)
                | AstKind::TSMethodSignature(_),
            ) => TypeParameterOwner::FunctionLike,
            Some(AstKind::Class(_)) => TypeParameterOwner::Class,
            Some(AstKind::TSInterfaceDeclaration(_)) => TypeParameterOwner::Interface,
            Some(AstKind::TSTypeAliasDeclaration(_)) => TypeParameterOwner::TypeAlias,
            _ => TypeParameterOwner::Other,
        };
        let context = ModifierContext {
            node: NodeKind::TypeParameter,
            parent: Parent::Other,
            parent_ambient: self.ambient_depth > 0,
            name_is_private: false,
            type_parameter_owner: owner,
        };
        self.examine_modifiers(&modifiers, &context);
    }

    /// tsc's `checkGrammarObjectLiteralExpression`: no modifier belongs on an
    /// object literal member except `async` on a method — TS1042 each; and
    /// `checkGrammarMethod`'s TS1184 on a method carrying anything else.
    fn check_object_member_modifiers(&mut self, property: &oxc_ast::ast::ObjectProperty<'_>) {
        let Some(modifiers) = modifiers::scan_modifiers(
            self.source_text,
            property.span.start,
            property.key.span().start,
            false,
        ) else {
            return;
        };
        for modifier in &modifiers {
            if modifier.kind == modifiers::ModifierKind::Async && property.method {
                continue;
            }
            self.push(1042, modifier.span, &[modifier.kind.text()]);
        }
        if property.method
            && property.kind == PropertyKind::Init
            && let Some(first) = modifiers.first()
            && !(modifiers.len() == 1 && first.kind == modifiers::ModifierKind::Async)
        {
            self.push(1184, first.span, &[]);
        }
    }

    /// Whether `kind` is what tsc calls an ambient module: a string-named
    /// module declaration or a global augmentation.
    fn is_ambient_module(kind: Option<AstKind<'a>>) -> bool {
        match kind {
            Some(AstKind::TSModuleDeclaration(module)) => {
                matches!(module.id, oxc_ast::ast::TSModuleDeclarationName::StringLiteral(_))
            }
            Some(AstKind::TSGlobalDeclaration(_)) => true,
            _ => false,
        }
    }

    /// tsc's `IsExternalModuleAugmentation` for an ambient module being
    /// entered: at the top level of an external module, or directly inside a
    /// top-level ambient module of a script.
    fn is_external_module_augmentation(&self) -> bool {
        let mut ancestors = self.ancestors_as_tsc();
        match ancestors.next() {
            None | Some(AstKind::Program(_)) => self.external_module,
            Some(AstKind::TSModuleBlock(_)) => {
                let grandparent = ancestors.next();
                Self::is_ambient_module(grandparent)
                    && matches!(ancestors.next(), None | Some(AstKind::Program(_)))
                    && !self.external_module
            }
            _ => false,
        }
    }

    /// tsc's `checkModuleDeclaration` for a `module`/`namespace` declaration:
    /// its position (TS1234/TS1235), modifiers, the `module` keyword on a
    /// namespace (TS1540), and where an ambient module may sit (TS2435).
    fn check_module_declaration(&mut self, declaration: &oxc_ast::ast::TSModuleDeclaration<'_>) {
        let (name_span, is_string_name) = match &declaration.id {
            oxc_ast::ast::TSModuleDeclarationName::Identifier(identifier) => (identifier.span, false),
            oxc_ast::ast::TSModuleDeclarationName::StringLiteral(literal) => (literal.span, true),
        };
        if is_string_name {
            self.check_ambient_module_export();
        }
        let parent = self.ancestors_as_tsc().next();
        let in_context = matches!(
            parent,
            None | Some(AstKind::Program(_) | AstKind::TSModuleBlock(_) | AstKind::TSModuleDeclaration(_))
        );
        if !in_context {
            let code = if is_string_name { 1234 } else { 1235 };
            let span = self.first_token(self.statement_start(declaration.span.start));
            self.push(code, span, &[]);
            return;
        }
        self.check_statement_modifiers(NodeKind::ModuleDeclaration, declaration.span);
        if !is_string_name && declaration.kind == oxc_ast::ast::TSModuleDeclarationKind::Module {
            self.push(1540, name_span, &[]);
        }
        if is_string_name && !self.is_external_module_augmentation() {
            let at_script_top_level =
                matches!(parent, None | Some(AstKind::Program(_))) && !self.external_module;
            if !at_script_top_level {
                self.push(2435, name_span, &[]);
            } else if let Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block)) = &declaration.body {
                self.check_ambient_module_relative_references(&block.body);
            }
        }
    }

    /// tsc's `checkExternalImportOrExportDeclaration` inside an ambient module
    /// declaration: a relative module name has nothing to resolve against
    /// (TS2439).
    fn check_ambient_module_relative_references(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            let specifier = match statement {
                Statement::ImportDeclaration(import) => Some(&import.source),
                Statement::ExportNamedDeclaration(export) => export.source.as_ref(),
                Statement::ExportAllDeclaration(export) => Some(&export.source),
                Statement::TSImportEqualsDeclaration(import) => match &import.module_reference {
                    oxc_ast::ast::TSModuleReference::ExternalModuleReference(reference) => {
                        Some(&reference.expression)
                    }
                    _ => None,
                },
                _ => None,
            };
            if specifier.is_some_and(|specifier| {
                // `isExternalModuleNameRelative`: relative or rooted.
                specifier.value.starts_with("./") || specifier.value.starts_with("../")
                    || specifier.value == "." || specifier.value == ".." || specifier.value.starts_with('/')
            }) {
                let span = self.first_token(statement.span().start);
                self.push(2439, span, &[]);
            }
        }
    }

    /// tsc's `checkModuleDeclaration` for `global { … }`: TS2670 without
    /// `declare` outside an ambient context, its position (TS1234), and
    /// TS2669 anywhere it does not augment an external module. An
    /// augmentation's body may not import or export (TS2666/TS2667).
    fn check_global_declaration(&mut self, declaration: &oxc_ast::ast::TSGlobalDeclaration<'_>) {
        self.check_ambient_module_export();
        if !declaration.declare && self.ambient_depth == 0 {
            self.push(2670, declaration.global_span, &[]);
        }
        let parent = self.ancestors_as_tsc().next();
        if !matches!(parent, None | Some(AstKind::Program(_) | AstKind::TSModuleBlock(_))) {
            let span = self.first_token(self.statement_start(declaration.span.start));
            self.push(1234, span, &[]);
            return;
        }
        if self.is_external_module_augmentation() {
            self.check_augmentation_elements(&declaration.body.body);
        } else {
            self.push(2669, declaration.global_span, &[]);
        }
    }

    /// The binder's TS2668: `export` on an ambient module or augmentation.
    fn check_ambient_module_export(&mut self) {
        if let Some(AstKind::ExportNamedDeclaration(export)) = self.stack.last() {
            let span = self.first_token(export.span.start);
            self.push(2668, span, &[]);
        }
    }

    /// The start of the statement a declaration belongs to: its `export`
    /// wrapper's, when there is one.
    fn statement_start(&self, start: u32) -> u32 {
        match self.stack.last() {
            Some(AstKind::ExportNamedDeclaration(export)) => export.span.start,
            _ => start,
        }
    }

    /// tsc's `checkModuleAugmentationElement`.
    fn check_augmentation_elements(&mut self, statements: &[Statement<'_>]) {
        for statement in statements {
            let code = match statement {
                Statement::ExportNamedDeclaration(export) if export.declaration.is_none() => 2666,
                Statement::ExportAllDeclaration(_) | Statement::TSExportAssignment(_) => 2666,
                Statement::ExportDefaultDeclaration(export)
                    if !matches!(
                        export.declaration,
                        oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(_)
                            | oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(_)
                            | oxc_ast::ast::ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
                    ) =>
                {
                    2666
                }
                Statement::ImportDeclaration(_) => 2667,
                Statement::TSImportEqualsDeclaration(import)
                    if matches!(
                        import.module_reference,
                        oxc_ast::ast::TSModuleReference::ExternalModuleReference(_)
                    ) =>
                {
                    2667
                }
                _ => continue,
            };
            let span = self.first_token(statement.span().start);
            self.push(code, span, &[]);
        }
    }

    /// tsc's `checkExportAssignment` for `export default <expression>`, and
    /// `checkGrammarModifiers` for a default-exported declaration: not
    /// inside a function or block (TS1258 / TS1184). The checker reports an
    /// export assignment inside a namespace (TS1319) itself, as tsc does.
    fn check_export_default(&mut self, declaration: &oxc_ast::ast::ExportDefaultDeclaration<'_>) {
        let is_declaration = matches!(
            declaration.declaration,
            oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(_)
                | oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(_)
                | oxc_ast::ast::ExportDefaultDeclarationKind::TSInterfaceDeclaration(_)
        );
        let mut ancestors = self.stack.iter().rev();
        let parent = ancestors.next();
        if !matches!(parent, None | Some(AstKind::Program(_) | AstKind::TSModuleBlock(_))) {
            let code = if is_declaration { 1184 } else { 1258 };
            let span = self.first_token(declaration.span.start);
            self.push(code, span, &[]);
            return;
        }
    }

    /// tsc's `checkExpressionWithTypeArguments`: an instantiation expression
    /// as the right operand of `instanceof` — TS2848.
    fn check_instanceof_instantiation(&mut self, binary: &oxc_ast::ast::BinaryExpression<'_>) {
        if binary.operator != oxc_syntax::operator::BinaryOperator::Instanceof {
            return;
        }
        let mut right = &binary.right;
        while let oxc_ast::ast::Expression::ParenthesizedExpression(inner) = right {
            right = &inner.expression;
        }
        if let oxc_ast::ast::Expression::TSInstantiationExpression(instantiation) = right {
            self.push(2848, instantiation.span, &[]);
        }
    }

    /// tsc's `checkGrammarForUseStrictSimpleParameterList`: a `"use strict"`
    /// prologue in a function whose parameter list is not simple — TS1346
    /// on each such parameter and TS1347 on the directive. tsc skips this
    /// below target ES2016, which this pass does not see: the driver drops
    /// them there.
    fn check_use_strict_parameters(
        &mut self,
        parameters: &oxc_ast::ast::FormalParameters<'_>,
        body: &oxc_ast::ast::FunctionBody<'_>,
    ) {
        let Some(directive) = body.directives.iter().find(|directive| {
            let span = directive.expression.span;
            matches!(
                self.source_text.get(span.start as usize..span.end as usize),
                Some("\"use strict\"" | "'use strict'")
            )
        }) else {
            return;
        };
        let mut non_simple: Vec<Span> = parameters
            .items
            .iter()
            .filter(|parameter| {
                parameter.initializer.is_some()
                    || !matches!(parameter.pattern, BindingPattern::BindingIdentifier(_))
            })
            .map(|parameter| parameter.span)
            .collect();
        if let Some(rest) = &parameters.rest {
            non_simple.push(rest.span);
        }
        if non_simple.is_empty() {
            return;
        }
        for span in non_simple {
            self.push(1346, span, &[]);
        }
        self.push(1347, directive.span, &[]);
    }
}

fn decorators_end(decorators: &[oxc_ast::ast::Decorator<'_>], start: u32) -> u32 {
    decorators
        .iter()
        .map(|decorator| decorator.span.end)
        .max()
        .unwrap_or(start)
        .max(start)
}

fn check_class_name(collector: &mut ContextCollector<'_, '_>, class: &Class<'_>) {
    if let Some(id) = class.id.as_ref() {
        collector.check_reserved_type_name(2414, id);
    }
}

fn signature_index_signatures<'s>(
    members: &'s [oxc_ast::ast::TSSignature<'s>],
) -> impl Iterator<Item = &'s oxc_ast::ast::TSIndexSignature<'s>> {
    members.iter().filter_map(|member| match member {
        oxc_ast::ast::TSSignature::TSIndexSignature(signature) => Some(&**signature),
        _ => None,
    })
}

fn keyword_text(ty: &oxc_ast::ast::TSType<'_>) -> Option<&'static str> {
    use oxc_ast::ast::TSType as T;
    Some(match ty {
        T::TSStringKeyword(_) => "string",
        T::TSNumberKeyword(_) => "number",
        T::TSBooleanKeyword(_) => "boolean",
        T::TSBigIntKeyword(_) => "bigint",
        T::TSSymbolKeyword(_) => "symbol",
        T::TSObjectKeyword(_) => "object",
        T::TSAnyKeyword(_) => "any",
        T::TSUnknownKeyword(_) => "unknown",
        T::TSNeverKeyword(_) => "never",
        T::TSVoidKeyword(_) => "void",
        T::TSUndefinedKeyword(_) => "undefined",
        T::TSNullKeyword(_) => "null",
        _ => return None,
    })
}

/// Whether `ty`, resolved eagerly, reads the declaration `name` with fewer
/// type arguments than reach the parameter at `index`.
fn default_reads_own_declaration(ty: &oxc_ast::ast::TSType<'_>, name: &str, index: usize) -> bool {
    use oxc_ast::ast::{TSTupleElement, TSType as T};
    let reads = |ty: &oxc_ast::ast::TSType<'_>| default_reads_own_declaration(ty, name, index);
    match ty {
        T::TSTypeReference(reference) => {
            let arguments = reference.type_arguments.as_ref().map_or(&[][..], |arguments| &arguments.params[..]);
            matches!(&reference.type_name, oxc_ast::ast::TSTypeName::IdentifierReference(id) if id.name == name)
                && arguments.len() <= index
                || arguments.iter().any(reads)
        }
        T::TSUnionType(union) => union.types.iter().any(reads),
        T::TSIntersectionType(intersection) => intersection.types.iter().any(reads),
        T::TSArrayType(array) => reads(&array.element_type),
        T::TSTupleType(tuple) => tuple.element_types.iter().any(|element| match element {
            TSTupleElement::TSOptionalType(optional) => reads(&optional.type_annotation),
            TSTupleElement::TSRestType(rest) => reads(&rest.type_annotation),
            TSTupleElement::TSNamedTupleMember(member) => member.element_type.as_ts_type().is_some_and(reads),
            other => other.as_ts_type().is_some_and(reads),
        }),
        T::TSTypeOperatorType(operator) => reads(&operator.type_annotation),
        T::TSIndexedAccessType(access) => reads(&access.object_type) || reads(&access.index_type),
        T::TSParenthesizedType(parenthesized) => reads(&parenthesized.type_annotation),
        _ => false,
    }
}

fn keywords_differ(left: &oxc_ast::ast::TSType<'_>, right: &oxc_ast::ast::TSType<'_>) -> bool {
    matches!((keyword_text(left), keyword_text(right)), (Some(l), Some(r)) if l != r)
}

/// Whether a namespace body declares a value (tsc's `getModuleInstanceState`
/// is not `NonInstantiated`): anything but interfaces, type aliases, and
/// namespaces that are themselves uninstantiated.
fn module_is_instantiated(declaration: &oxc_ast::ast::TSModuleDeclaration<'_>) -> bool {
    match &declaration.body {
        None => false,
        Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleDeclaration(inner)) => module_is_instantiated(inner),
        Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block)) => block.body.iter().any(|statement| {
            let declaration = match statement {
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(declaration) => declaration,
                    None => return false,
                },
                Statement::TSInterfaceDeclaration(_) | Statement::TSTypeAliasDeclaration(_) => return false,
                Statement::TSModuleDeclaration(inner) => return module_is_instantiated(inner),
                _ => return true,
            };
            match declaration {
                oxc_ast::ast::Declaration::TSInterfaceDeclaration(_)
                | oxc_ast::ast::Declaration::TSTypeAliasDeclaration(_) => false,
                oxc_ast::ast::Declaration::TSModuleDeclaration(inner) => module_is_instantiated(inner),
                _ => true,
            }
        }),
    }
}

fn declaration_is_value(declaration: &oxc_ast::ast::Declaration<'_>) -> bool {
    use oxc_ast::ast::Declaration as D;
    match declaration {
        D::VariableDeclaration(_) | D::FunctionDeclaration(_) | D::ClassDeclaration(_) | D::TSEnumDeclaration(_) => true,
        D::TSModuleDeclaration(module) => module_is_instantiated(module),
        _ => false,
    }
}

fn statement_declares_value(statement: &Statement<'_>, name: &str) -> bool {
    let declaration = match statement {
        Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
        other => other.as_declaration(),
    };
    let Some(declaration) = declaration else {
        return false;
    };
    use oxc_ast::ast::Declaration as D;
    let declares = match declaration {
        D::VariableDeclaration(d) => d.declarations.iter().any(|declarator| {
            let mut names = Vec::new();
            collect_binding_names(&declarator.id, &mut names);
            names.iter().any(|(declared, _)| *declared == name)
        }),
        D::FunctionDeclaration(d) => d.id.as_ref().is_some_and(|id| id.name == name),
        D::ClassDeclaration(d) => d.id.as_ref().is_some_and(|id| id.name == name),
        D::TSEnumDeclaration(d) => d.id.name == name,
        D::TSModuleDeclaration(d) => {
            matches!(&d.id, oxc_ast::ast::TSModuleDeclarationName::Identifier(id) if id.name == name)
                && module_is_instantiated(d)
        }
        _ => false,
    };
    declares
}

/// The value names an expression reads as it runs. A nested function, arrow,
/// or class body runs later (tsc's `withinDeferredContext`), and a type
/// annotation reads no values.
#[derive(Default)]
struct EagerReferences {
    found: Vec<(String, Span)>,
    deferred_depth: usize,
    /// A class expression with a static property was read eagerly.
    static_class_property: bool,
}

impl<'a> Visit<'a> for EagerReferences {
    fn visit_identifier_reference(&mut self, identifier: &oxc_ast::ast::IdentifierReference<'a>) {
        if self.deferred_depth == 0 {
            self.found.push((identifier.name.to_string(), identifier.span));
        }
    }
    fn visit_function(&mut self, function: &oxc_ast::ast::Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        self.deferred_depth += 1;
        oxc_ast_visit::walk::walk_function(self, function, flags);
        self.deferred_depth -= 1;
    }
    fn visit_arrow_function_expression(&mut self, arrow: &oxc_ast::ast::ArrowFunctionExpression<'a>) {
        self.deferred_depth += 1;
        oxc_ast_visit::walk::walk_arrow_function_expression(self, arrow);
        self.deferred_depth -= 1;
    }
    // A class's heritage is evaluated with it; its members are not read here.
    fn visit_class(&mut self, class: &Class<'a>) {
        if self.deferred_depth == 0
            && class.body.body.iter().any(|element| {
                matches!(element, oxc_ast::ast::ClassElement::PropertyDefinition(property) if property.r#static)
            })
        {
            self.static_class_property = true;
        }
        if let Some(base) = &class.super_class {
            self.visit_expression(base);
        }
    }
    fn visit_ts_type_annotation(&mut self, _: &oxc_ast::ast::TSTypeAnnotation<'a>) {}
    fn visit_ts_type(&mut self, _: &oxc_ast::ast::TSType<'a>) {}
}

/// The names a type annotation reads with `typeof` while its own type is
/// being resolved: through unions, intersections, arrays, tuples, operators,
/// indexed accesses and type arguments, but not into an object type's
/// members or a signature, which tsc resolves on demand.
fn eager_type_queries<'t>(ty: &'t oxc_ast::ast::TSType<'_>, out: &mut Vec<&'t str>) {
    use oxc_ast::ast::TSType as T;
    match ty {
        T::TSTypeQuery(query) => match &query.expr_name {
            oxc_ast::ast::TSTypeQueryExprName::IdentifierReference(identifier) => {
                out.push(identifier.name.as_str());
            }
            oxc_ast::ast::TSTypeQueryExprName::QualifiedName(qualified) => {
                let mut left = &qualified.left;
                while let oxc_ast::ast::TSTypeName::QualifiedName(inner) = left {
                    left = &inner.left;
                }
                if let oxc_ast::ast::TSTypeName::IdentifierReference(identifier) = left {
                    out.push(identifier.name.as_str());
                }
            }
            _ => {}
        },
        T::TSUnionType(union) => union.types.iter().for_each(|member| eager_type_queries(member, out)),
        T::TSIntersectionType(intersection) => {
            intersection.types.iter().for_each(|member| eager_type_queries(member, out));
        }
        T::TSArrayType(array) => eager_type_queries(&array.element_type, out),
        T::TSParenthesizedType(inner) => eager_type_queries(&inner.type_annotation, out),
        T::TSTypeOperatorType(operator) => eager_type_queries(&operator.type_annotation, out),
        T::TSIndexedAccessType(access) => {
            eager_type_queries(&access.object_type, out);
            eager_type_queries(&access.index_type, out);
        }
        T::TSTupleType(tuple) => {
            for element in &tuple.element_types {
                if let Some(ty) = element.as_ts_type() {
                    eager_type_queries(ty, out);
                }
            }
        }
        T::TSTypeReference(reference) => {
            if let Some(arguments) = &reference.type_arguments {
                arguments.params.iter().for_each(|argument| eager_type_queries(argument, out));
            }
        }
        _ => {}
    }
}

/// What a type parameter stands for while an alias body is classified for
/// TS2637: a type variable of the annotated alias, or the argument a
/// reference to another alias supplies.
#[derive(Clone, Copy)]
struct AliasArgument {
    /// [`declared_type_is_not_anonymous`] of the argument.
    not_anonymous: Option<bool>,
    /// A type variable, which `isGenericObjectType` and `isGenericIndexType`
    /// hold for.
    variable: bool,
}

/// How many alias references a TS2637 classification follows; a circular
/// chain is TS2456's to report.
const ALIAS_CHAIN_LIMIT: usize = 8;

/// tsc's TS2637 condition, `getDeclaredTypeOfSymbol(alias)` being neither
/// anonymous nor mapped, where the syntax settles it. `statements` is the
/// file's top level when the alias is declared there: a name no type
/// parameter binds then resolves among them. Any other reference is left
/// alone, since it may name an object type.
fn alias_is_not_anonymous(
    alias: &oxc_ast::ast::TSTypeAliasDeclaration<'_>,
    statements: Option<&[Statement<'_>]>,
) -> bool {
    let variable = AliasArgument { not_anonymous: Some(true), variable: true };
    let scope: Vec<(&str, AliasArgument)> = alias
        .type_parameters
        .iter()
        .flat_map(|parameters| parameters.params.iter())
        .map(|parameter| (parameter.name.name.as_str(), variable))
        .collect();
    declared_type_is_not_anonymous(&alias.type_annotation, &scope, statements, 0) == Some(true)
}

/// Whether the type `ty` declares is neither anonymous nor mapped
/// (`Some(true)`), is one of them (`Some(false)`), or cannot be told without
/// resolving it (`None`). `scope` binds the type parameters in effect.
fn declared_type_is_not_anonymous(
    ty: &oxc_ast::ast::TSType<'_>,
    scope: &[(&str, AliasArgument)],
    statements: Option<&[Statement<'_>]>,
    depth: usize,
) -> Option<bool> {
    use oxc_ast::ast::TSType as T;
    match skip_type_parentheses(ty) {
        T::TSTypeLiteral(_) | T::TSFunctionType(_) | T::TSConstructorType(_) | T::TSMappedType(_) => Some(false),
        T::TSTypeReference(reference) => {
            let oxc_ast::ast::TSTypeName::IdentifierReference(name) = &reference.type_name else {
                return None;
            };
            if let Some((_, argument)) = scope.iter().find(|(parameter, _)| *parameter == name.name.as_str()) {
                return if reference.type_arguments.is_none() { argument.not_anonymous } else { None };
            }
            match file_type_declaration(statements?, name.name.as_str())? {
                FileTypeDeclaration::ClassOrInterface => Some(true),
                // `getTypeAliasInstantiation`: the target's declared type with
                // its type parameters mapped to these arguments.
                FileTypeDeclaration::Alias(target) => {
                    if depth == ALIAS_CHAIN_LIMIT {
                        return None;
                    }
                    let parameters = target.type_parameters.as_ref().map_or(&[][..], |list| &list.params[..]);
                    let arguments = reference.type_arguments.as_ref().map_or(&[][..], |list| &list.params[..]);
                    if parameters.len() != arguments.len() {
                        return None;
                    }
                    let bound: Vec<(&str, AliasArgument)> = parameters
                        .iter()
                        .zip(arguments)
                        .map(|(parameter, argument)| {
                            let argument = AliasArgument {
                                not_anonymous: declared_type_is_not_anonymous(argument, scope, statements, depth + 1),
                                variable: names_type_variable(argument, scope),
                            };
                            (parameter.name.name.as_str(), argument)
                        })
                        .collect();
                    declared_type_is_not_anonymous(&target.type_annotation, &bound, statements, depth + 1)
                }
            }
        }
        // `getIndexedAccessType` defers on a generic object or index type,
        // `getConditionalType` on a generic check or extends type.
        T::TSIndexedAccessType(access) => (names_type_variable(&access.object_type, scope)
            || names_type_variable(&access.index_type, scope)
            || matches!(skip_type_parentheses(&access.index_type), T::TSTypeOperatorType(operator)
                if operator.operator == oxc_ast::ast::TSTypeOperatorOperator::Keyof
                    && names_type_variable(&operator.type_annotation, scope)))
        .then_some(true),
        T::TSConditionalType(conditional) => (names_type_variable(&conditional.check_type, scope)
            || names_type_variable(&conditional.extends_type, scope))
        .then_some(true),
        // `getUnionType` drops `never`; a lone survivor is the type itself.
        T::TSUnionType(union) => {
            let mut members = union
                .types
                .iter()
                .filter(|member| !matches!(skip_type_parentheses(member), T::TSNeverKeyword(_)));
            match (members.next(), members.next()) {
                (Some(member), None) => declared_type_is_not_anonymous(member, scope, statements, depth),
                _ => Some(true),
            }
        }
        T::TSTypeOperatorType(_)
        | T::TSStringKeyword(_)
        | T::TSNumberKeyword(_)
        | T::TSBooleanKeyword(_)
        | T::TSBigIntKeyword(_)
        | T::TSSymbolKeyword(_)
        | T::TSObjectKeyword(_)
        | T::TSAnyKeyword(_)
        | T::TSUnknownKeyword(_)
        | T::TSNeverKeyword(_)
        | T::TSVoidKeyword(_)
        | T::TSUndefinedKeyword(_)
        | T::TSNullKeyword(_)
        | T::TSLiteralType(_)
        | T::TSTemplateLiteralType(_)
        | T::TSIntersectionType(_)
        | T::TSArrayType(_)
        | T::TSTupleType(_) => Some(true),
        _ => None,
    }
}

fn skip_type_parentheses<'t, 'a>(mut ty: &'t oxc_ast::ast::TSType<'a>) -> &'t oxc_ast::ast::TSType<'a> {
    while let oxc_ast::ast::TSType::TSParenthesizedType(inner) = ty {
        ty = &inner.type_annotation;
    }
    ty
}

/// A bare reference to a type parameter that stands for a type variable.
fn names_type_variable(ty: &oxc_ast::ast::TSType<'_>, scope: &[(&str, AliasArgument)]) -> bool {
    let oxc_ast::ast::TSType::TSTypeReference(reference) = skip_type_parentheses(ty) else {
        return false;
    };
    let oxc_ast::ast::TSTypeName::IdentifierReference(name) = &reference.type_name else {
        return false;
    };
    reference.type_arguments.is_none()
        && scope
            .iter()
            .find(|(parameter, _)| *parameter == name.name.as_str())
            .is_some_and(|(_, argument)| argument.variable)
}

/// The type meaning of a name among a file's top-level declarations.
enum FileTypeDeclaration<'s, 'a> {
    /// Interfaces and classes, merged or not: an interface or class type, or
    /// a reference to one.
    ClassOrInterface,
    Alias(&'s oxc_ast::ast::TSTypeAliasDeclaration<'a>),
}

/// What `name` declares among `statements`, or `None` where nothing does or an
/// import or a declaration that does not merge makes it more than the syntax.
fn file_type_declaration<'s, 'a>(statements: &'s [Statement<'a>], name: &str) -> Option<FileTypeDeclaration<'s, 'a>> {
    use oxc_ast::ast::Declaration as D;
    let mut found = None;
    for statement in statements {
        if let Statement::ImportDeclaration(import) = statement
            && import.specifiers.iter().flatten().any(|specifier| specifier.local().name.as_str() == name)
        {
            return None;
        }
        let declaration = match statement {
            Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
            other => other.as_declaration(),
        };
        let current = match declaration {
            Some(D::TSInterfaceDeclaration(interface)) if interface.id.name.as_str() == name => {
                FileTypeDeclaration::ClassOrInterface
            }
            Some(D::ClassDeclaration(class)) if class.id.as_ref().is_some_and(|id| id.name.as_str() == name) => {
                FileTypeDeclaration::ClassOrInterface
            }
            Some(D::TSTypeAliasDeclaration(alias)) if alias.id.name.as_str() == name => FileTypeDeclaration::Alias(alias),
            Some(D::TSImportEqualsDeclaration(import)) if import.id.name.as_str() == name => return None,
            _ => continue,
        };
        found = match (found, current) {
            (None, current) => Some(current),
            (Some(FileTypeDeclaration::ClassOrInterface), FileTypeDeclaration::ClassOrInterface) => {
                Some(FileTypeDeclaration::ClassOrInterface)
            }
            _ => return None,
        };
    }
    found
}

/// A name tsc treats as static: a string or numeric literal, a signed number,
/// or a template without substitutions (`isDynamicName` is false for these).
fn is_literal_name(expression: &oxc_ast::ast::Expression<'_>) -> bool {
    use oxc_ast::ast::Expression as E;
    match expression {
        E::StringLiteral(_) | E::NumericLiteral(_) => true,
        E::TemplateLiteral(template) => template.expressions.is_empty(),
        E::UnaryExpression(unary) => {
            matches!(
                unary.operator,
                oxc_syntax::operator::UnaryOperator::UnaryNegation | oxc_syntax::operator::UnaryOperator::UnaryPlus
            ) && matches!(unary.argument, E::NumericLiteral(_))
        }
        _ => false,
    }
}

/// tsc's `isEntityNameExpression`: an identifier, or a property access on one.
fn is_entity_name_expression(expression: &oxc_ast::ast::Expression<'_>) -> bool {
    use oxc_ast::ast::Expression as E;
    match expression {
        E::Identifier(_) => true,
        E::StaticMemberExpression(member) => is_entity_name_expression(&member.object),
        _ => false,
    }
}

fn statements_declare_block_scoped(statements: &[Statement<'_>], name: &str) -> bool {
    statements.iter().any(|statement| {
        let declaration = match statement {
            Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
            other => other.as_declaration(),
        };
        matches!(
            declaration,
            Some(oxc_ast::ast::Declaration::VariableDeclaration(declaration))
                if variable_declaration_declares_block_scoped(declaration, name)
        )
    })
}

fn variable_declaration_declares_block_scoped(
    declaration: &oxc_ast::ast::VariableDeclaration<'_>,
    name: &str,
) -> bool {
    declaration.kind != VariableDeclarationKind::Var
        && declaration.declarations.iter().any(|declarator| {
            declarator
                .id
                .get_binding_identifiers()
                .iter()
                .any(|identifier| identifier.name == name)
        })
}

/// tsc's `IsExternalModuleNameRelative`: `./`, `../`, or a rooted path.
fn is_external_module_name_relative(name: &str) -> bool {
    let relative = name == "." || name == ".." || name.starts_with("./") || name.starts_with("../")
        || name.starts_with(".\\") || name.starts_with("..\\");
    let rooted = name.starts_with('/')
        || name.starts_with('\\')
        || (name.len() >= 2
            && name.as_bytes()[0].is_ascii_alphabetic()
            && name.as_bytes()[1] == b':');
    relative || rooted
}

/// Variables whose own annotations reach each other through `typeof` in a
/// position tsc resolves eagerly — TS2502 on each one in the cycle. A
/// member of an object type or a signature is resolved on demand, so a
/// `typeof` there does not close a cycle. tsc types each one `errorType`.
pub(crate) fn self_referencing_annotations<'s>(statements: &'s [Statement<'_>]) -> Vec<(&'s str, Span)> {
    let mut variables: Vec<(&str, Span, Vec<&str>)> = Vec::new();
    // A redeclared variable takes its type from its first declaration
    // (`valueDeclaration`); a later `var p: typeof p` reads that type.
    let mut declared: Vec<&str> = Vec::new();
    for statement in statements {
        let declaration = match statement {
            Statement::VariableDeclaration(declaration) => Some(&**declaration),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(oxc_ast::ast::Declaration::VariableDeclaration(declaration)) => Some(&**declaration),
                _ => None,
            },
            _ => None,
        };
        let Some(declaration) = declaration else {
            continue;
        };
        for declarator in &declaration.declarations {
            let BindingPattern::BindingIdentifier(id) = &declarator.id else {
                continue;
            };
            if declared.contains(&id.name.as_str()) {
                continue;
            }
            declared.push(id.name.as_str());
            let Some(annotation) = declarator.type_annotation.as_ref() else {
                continue;
            };
            let mut queried = Vec::new();
            eager_type_queries(&annotation.type_annotation, &mut queried);
            if !queried.is_empty() {
                variables.push((id.name.as_str(), id.span, queried));
            }
        }
    }
    let mut found = Vec::new();
    for (name, span, _) in &variables {
        // Reachable from itself through the query edges.
        let mut stack: Vec<&str> = vec![name];
        let mut seen: Vec<&str> = Vec::new();
        let mut cyclic = false;
        while let Some(current) = stack.pop() {
            let Some((_, _, edges)) = variables.iter().find(|(other, _, _)| *other == current) else {
                continue;
            };
            for edge in edges {
                if edge == name {
                    cyclic = true;
                } else if !seen.contains(edge) {
                    seen.push(edge);
                    stack.push(edge);
                }
            }
        }
        if cyclic {
            found.push((*name, *span));
        }
    }
    found
}

/// Where the name of every variable [`self_referencing_annotations`] finds
/// starts, over each statement list the grammar walk checks.
pub(crate) fn circular_annotation_starts(program: &oxc_ast::ast::Program<'_>) -> Vec<u32> {
    struct Collector(Vec<u32>);
    impl Collector {
        fn statements(&mut self, statements: &[Statement<'_>]) {
            self.0
                .extend(self_referencing_annotations(statements).into_iter().map(|(_, span)| span.start));
        }
    }
    impl<'a> Visit<'a> for Collector {
        fn visit_program(&mut self, program: &oxc_ast::ast::Program<'a>) {
            self.statements(&program.body);
            oxc_ast_visit::walk::walk_program(self, program);
        }
        fn visit_block_statement(&mut self, block: &oxc_ast::ast::BlockStatement<'a>) {
            self.statements(&block.body);
            oxc_ast_visit::walk::walk_block_statement(self, block);
        }
        fn visit_function_body(&mut self, body: &oxc_ast::ast::FunctionBody<'a>) {
            self.statements(&body.statements);
            oxc_ast_visit::walk::walk_function_body(self, body);
        }
        fn visit_ts_module_block(&mut self, block: &oxc_ast::ast::TSModuleBlock<'a>) {
            self.statements(&block.body);
            oxc_ast_visit::walk::walk_ts_module_block(self, block);
        }
    }
    let mut collector = Collector(Vec::new());
    collector.visit_program(program);
    collector.0.sort_unstable();
    collector.0
}
