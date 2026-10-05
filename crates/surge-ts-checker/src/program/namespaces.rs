//! Which names are namespaces, whether each has a value side, and what each
//! exports.
//!
//! tsc's binder declares a namespace whose body holds only types as a
//! `NamespaceModule` with no value meaning (`getModuleInstanceState`), so a
//! value reference to it fails to resolve and reports TS2708. surge binds
//! namespaces through its value tables, which cannot say "this name is a
//! namespace" once the value is left out; this registry keeps that answer.
//! Namespaces are only declared at file, namespace or `declare global` level,
//! so a file scope plus the global scope covers every lookup.

use std::sync::Arc;

use surge_ts_syntax::{
    ParsedExportDeclaration, ParsedImportKind, ParsedNamespaceDeclaration, ParsedStatement,
};
use surge_ts_types::fx::FxHashMap;

use super::ParsedProgramFile;
use crate::context::CheckerContext;

/// The meanings an exported member carries (tsc's `SymbolFlags` groups).
pub(crate) const MEANING_TYPE: u8 = 1;
pub(crate) const MEANING_VALUE: u8 = 2;
pub(crate) const MEANING_NAMESPACE: u8 = 4;

#[derive(Debug, Default, Clone)]
pub(crate) struct NamespaceInfo {
    /// Whether any block gives the namespace a value side.
    pub(crate) instantiated: bool,
    /// Every exported member by name, with the meanings it carries.
    pub(crate) exports: FxHashMap<Arc<str>, u8>,
    /// Whether `exports` is the whole export list: a re-export clause, an
    /// exported import alias or a statement surge does not lower could add
    /// members it cannot see.
    pub(crate) complete: bool,
    /// An `enum`: its members are exports for a qualified name (`E.A`), but it
    /// is not a namespace for the namespace-as-value/type checks.
    pub(crate) is_enum: bool,
}

/// Namespace names (dotted for nested ones) mapped to what their blocks
/// declare, merged across blocks.
#[derive(Debug, Default)]
pub(crate) struct NamespaceRegistry {
    by_file: FxHashMap<Arc<str>, FxHashMap<Arc<str>, NamespaceInfo>>,
    global: FxHashMap<Arc<str>, NamespaceInfo>,
}

impl NamespaceRegistry {
    /// The namespace `name` visible from `file_name`.
    pub(crate) fn info(&self, file_name: &str, name: &str) -> Option<&NamespaceInfo> {
        self.by_file
            .get(file_name)
            .and_then(|names| names.get(name))
            .or_else(|| self.global.get(name))
    }

    /// The namespace `name` declared in `file_name` itself, not a global one.
    pub(crate) fn local_info(&self, file_name: &str, name: &str) -> Option<&NamespaceInfo> {
        self.by_file.get(file_name).and_then(|names| names.get(name))
    }

    /// The namespace `name` declared at the top of the module whose file name,
    /// without its TypeScript extension, is `module_path`.
    pub(crate) fn info_in_module(&self, module_path: &str, name: &str) -> Option<&NamespaceInfo> {
        self.by_file.iter().find_map(|(file, names)| {
            (crate::modules::strip_typescript_extension(file) == module_path)
                .then(|| names.get(name))
                .flatten()
        })
    }

    /// Whether `name` is declared as a *global* namespace — the only place
    /// tsc looks for `JSX`, which a module's own `declare namespace JSX` is
    /// therefore not.
    pub(crate) fn is_global(&self, name: &str) -> bool {
        self.global.contains_key(name)
    }

    /// `Some(instantiated)` when `name` is a namespace visible from `file_name`.
    pub(crate) fn lookup(&self, file_name: &str, name: &str) -> Option<bool> {
        self.info(file_name, name)
            .filter(|info| !info.is_enum)
            .map(|info| info.instantiated)
    }
}

pub(crate) fn collect_namespace_registry(
    parsed_files: &[ParsedProgramFile],
    ctx: &mut CheckerContext,
) {
    let mut registry = NamespaceRegistry::default();
    for parsed_file in parsed_files {
        let ambient_file = parsed_file.file_kind.is_declaration();
        let mut file_names = FxHashMap::default();
        let top_level = if parsed_file.is_module {
            &mut file_names
        } else {
            &mut registry.global
        };
        record_namespaces(&parsed_file.statements, "", ambient_file, top_level);
        for statement in &parsed_file.statements {
            record_global_block_namespaces(statement, &mut registry.global);
        }
        if !file_names.is_empty() {
            registry
                .by_file
                .insert(Arc::from(parsed_file.file_name.as_str()), file_names);
        }
    }
    ctx.namespace_registry = Arc::new(registry);
}

fn record_namespaces(
    statements: &[ParsedStatement],
    prefix: &str,
    ambient: bool,
    names: &mut FxHashMap<Arc<str>, NamespaceInfo>,
) {
    record_enums(statements, prefix, names);
    for statement in statements {
        let Some(namespace) = namespace_of(statement) else {
            continue;
        };
        // The parser nests `namespace A.B { … }` as `A` holding a namespace
        // already named `A.B`.
        let relative = qualified_below(&namespace.name, prefix).unwrap_or(&namespace.name);
        let name = if prefix.is_empty() {
            relative.to_string()
        } else {
            format!("{prefix}.{relative}")
        };
        let ambient = ambient || namespace.is_declare;
        let instantiated = is_instantiated_namespace(namespace);
        // `namespace A.B { … }` also declares `A`, which exports `B`.
        let mut outer = prefix.to_string();
        let segments: Vec<&str> = relative.split('.').collect();
        for (index, segment) in segments.iter().enumerate().take(segments.len() - 1) {
            let head = if outer.is_empty() {
                (*segment).to_string()
            } else {
                format!("{outer}.{segment}")
            };
            let entry = names
                .entry(Arc::from(head.as_str()))
                .or_insert_with(|| NamespaceInfo {
                    complete: true,
                    ..NamespaceInfo::default()
                });
            entry.instantiated |= instantiated;
            let member = segments[index + 1];
            *entry.exports.entry(Arc::from(member)).or_default() |=
                MEANING_NAMESPACE | if instantiated { MEANING_VALUE } else { 0 };
            outer = head;
        }
        let entry = names
            .entry(Arc::from(name.as_str()))
            .or_insert_with(|| NamespaceInfo {
                complete: true,
                ..NamespaceInfo::default()
            });
        entry.instantiated |= instantiated;
        record_exports(&namespace.statements, &name, ambient, entry);
        record_namespaces(&namespace.statements, &name, ambient, names);
    }
}

/// Every `enum` declared among `statements`, with its members as exports.
/// An enum is lowered to an alias of its own name plus one alias per member
/// (`E.A`), each remembering the enum.
fn record_enums(statements: &[ParsedStatement], prefix: &str, names: &mut FxHashMap<Arc<str>, NamespaceInfo>) {
    let qualify = |name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        }
    };
    for statement in statements {
        let ParsedStatement::TypeAliasDeclaration(alias) = peel_export(statement) else {
            continue;
        };
        let Some(enum_name) = alias.enum_name.as_deref() else {
            continue;
        };
        let entry = names
            .entry(Arc::from(qualify(enum_name).as_str()))
            .or_insert_with(|| NamespaceInfo {
                instantiated: true,
                complete: true,
                is_enum: true,
                ..NamespaceInfo::default()
            });
        if let Some(member) = alias
            .name
            .strip_prefix(enum_name)
            .and_then(|rest| rest.strip_prefix('.'))
        {
            *entry.exports.entry(Arc::from(member)).or_default() |= MEANING_TYPE | MEANING_VALUE;
        }
    }
}

/// tsc's export table for one namespace block. In an ambient namespace every
/// declaration is exported; elsewhere only the `export`ed ones are.
fn record_exports(
    statements: &[ParsedStatement],
    namespace_name: &str,
    ambient: bool,
    info: &mut NamespaceInfo,
) {
    for statement in statements {
        // The inner half of `namespace A.B` is exported from `A` implicitly.
        if let ParsedStatement::NamespaceDeclaration(inner) = statement
            && let Some(relative) = qualified_below(&inner.name, namespace_name)
        {
            let head = relative.split('.').next().unwrap_or(relative);
            *info.exports.entry(Arc::from(head)).or_default() |= MEANING_NAMESPACE
                | if is_instantiated_namespace(inner) {
                    MEANING_VALUE
                } else {
                    0
                };
            continue;
        }
        let (exported, declaration) = match statement {
            ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                ParsedExportDeclaration::Statement { declaration, .. } => {
                    (true, declaration.as_ref())
                }
                ParsedExportDeclaration::Empty { .. } => continue,
                _ => {
                    info.complete = false;
                    continue;
                }
            },
            other => (ambient, other),
        };
        let (name, meanings) = match declaration {
            ParsedStatement::InterfaceDeclaration(interface) => {
                (interface.name.as_str(), MEANING_TYPE)
            }
            ParsedStatement::TypeAliasDeclaration(alias) => match &alias.enum_name {
                Some(_) => (
                    alias.name.as_str(),
                    MEANING_TYPE | MEANING_VALUE | MEANING_NAMESPACE,
                ),
                None => (alias.name.as_str(), MEANING_TYPE),
            },
            ParsedStatement::ClassDeclaration(class) => {
                (class.name.as_str(), MEANING_TYPE | MEANING_VALUE)
            }
            ParsedStatement::FunctionDeclaration(function) => {
                (function.name.as_str(), MEANING_VALUE)
            }
            ParsedStatement::VariableDeclaration(variable) => {
                (variable.name.as_str(), MEANING_VALUE)
            }
            ParsedStatement::NamespaceDeclaration(namespace) => (
                namespace.name.as_str(),
                MEANING_NAMESPACE
                    | if is_instantiated_namespace(namespace) {
                        MEANING_VALUE
                    } else {
                        0
                    },
            ),
            ParsedStatement::ImportDeclaration(_) => {
                if exported {
                    info.complete = false;
                }
                continue;
            }
            ParsedStatement::UnsupportedDeclaration { .. } => {
                info.complete = false;
                continue;
            }
            _ => continue,
        };
        if !exported {
            continue;
        }
        // An enum's lowered member aliases (`E.A`) and a dotted namespace are
        // exported under their first segment.
        let head = name.split('.').next().unwrap_or(name);
        *info.exports.entry(Arc::from(head)).or_default() |= meanings;
    }
}

/// `declare global { namespace X {} }`, including the
/// `declare module "x" { global { … } }` nesting.
fn record_global_block_namespaces(
    statement: &ParsedStatement,
    global: &mut FxHashMap<Arc<str>, NamespaceInfo>,
) {
    let ParsedStatement::DeclareModuleDeclaration(module) = peel_export(statement) else {
        return;
    };
    if module.module_specifier == "global" {
        record_namespaces(&module.statements, "", true, global);
    } else {
        for inner in &module.statements {
            record_global_block_namespaces(inner, global);
        }
    }
}

/// `name` with the `prefix.` it is already qualified by removed.
fn qualified_below<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return None;
    }
    name.strip_prefix(prefix)?.strip_prefix('.')
}

fn peel_export(statement: &ParsedStatement) -> &ParsedStatement {
    match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
            _ => statement,
        },
        _ => statement,
    }
}

fn namespace_of(statement: &ParsedStatement) -> Option<&ParsedNamespaceDeclaration> {
    match peel_export(statement) {
        ParsedStatement::NamespaceDeclaration(namespace) => Some(namespace),
        _ => None,
    }
}

/// tsc's `getModuleInstanceState` != `NonInstantiated`. A `const enum` makes a
/// namespace `ConstEnumOnly`, which still declares a value module.
pub(crate) fn is_instantiated_namespace(namespace: &ParsedNamespaceDeclaration) -> bool {
    namespace
        .statements
        .iter()
        .any(|statement| is_instantiating_statement(statement, &namespace.statements))
}

fn is_instantiating_statement(statement: &ParsedStatement, siblings: &[ParsedStatement]) -> bool {
    match statement {
        ParsedStatement::InterfaceDeclaration(_) => false,
        // An `enum` is lowered to an alias that remembers it.
        ParsedStatement::TypeAliasDeclaration(alias) => alias.enum_name.is_some(),
        ParsedStatement::ImportDeclaration(_) => false,
        ParsedStatement::NamespaceDeclaration(namespace) => is_instantiated_namespace(namespace),
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => match declaration.as_ref() {
                ParsedStatement::ImportDeclaration(_) => true,
                declaration => is_instantiating_statement(declaration, siblings),
            },
            ParsedExportDeclaration::Named {
                module_specifier: None,
                specifiers,
                ..
            } => specifiers
                .iter()
                .any(|specifier| is_instantiated_alias_target(&specifier.local_name, siblings)),
            ParsedExportDeclaration::Empty { .. } => false,
            _ => true,
        },
        _ => true,
    }
}

/// tsc's `getModuleInstanceStateForAliasTarget`, over the enclosing block only:
/// a name it cannot locate is assumed to be a value.
fn is_instantiated_alias_target(name: &str, siblings: &[ParsedStatement]) -> bool {
    let mut found = false;
    for statement in siblings {
        let declaration = peel_export(statement);
        // A re-exported `import x = …` alias is ambiguous to tsc, which treats
        // it as instantiated.
        if let ParsedStatement::ImportDeclaration(import) = declaration
            && let ParsedImportKind::Equals { local_name, .. } = &import.kind
            && local_name == name
        {
            return true;
        }
        if !declares_name(declaration, name) {
            continue;
        }
        found = true;
        if is_instantiating_statement(declaration, siblings) {
            return true;
        }
    }
    !found
}

fn declares_name(statement: &ParsedStatement, name: &str) -> bool {
    match statement {
        ParsedStatement::InterfaceDeclaration(interface) => interface.name == name,
        ParsedStatement::TypeAliasDeclaration(alias) => alias.name == name,
        ParsedStatement::NamespaceDeclaration(namespace) => namespace.name == name,
        ParsedStatement::ClassDeclaration(class) => class.name == name,
        ParsedStatement::FunctionDeclaration(function) => function.name == name,
        ParsedStatement::VariableDeclaration(variable) => variable.name == name,
        _ => false,
    }
}

/// tsc's `checkModuleDeclaration` merge placement across global scripts: a
/// non-ambient instantiated namespace whose merged symbol's first non-ambient
/// class or implemented function lives in another file is TS2433 at its name.
/// A namespace body's exported members merge across its blocks, and so across
/// files: `namespace A { export namespace P {} }` meets `namespace A { export
/// class P {} }` written elsewhere. Declarations merge in program file order.
/// The same-file case (TS2434) is the parser's, and gives way here when the
/// first such declaration is in an earlier file.
pub(crate) fn report_cross_file_namespace_merges(parsed_files: &mut [ParsedProgramFile]) {
    let is_global_script =
        |file: &ParsedProgramFile| !file.is_module && !file.file_kind.is_declaration();
    let mut first_class_or_function: FxHashMap<String, usize> = FxHashMap::default();
    for (index, file) in parsed_files.iter().enumerate() {
        if is_global_script(file) {
            collect_merged_classes_and_functions(
                &file.statements,
                None,
                index,
                &mut first_class_or_function,
            );
        }
    }
    let mut reports: Vec<(usize, surge_ts_syntax::TextSpan)> = Vec::new();
    for (index, file) in parsed_files.iter().enumerate() {
        if is_global_script(file) {
            collect_cross_file_namespace_merges(
                &file.statements,
                None,
                index,
                &first_class_or_function,
                &mut reports,
            );
        }
    }
    for (index, span) in reports {
        let findings = &mut parsed_files[index].grammar_diagnostics;
        findings.retain(|finding| {
            !(finding.kind == surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(2434) && finding.span == span)
        });
        findings.push(surge_ts_syntax::ParsedGrammarDiagnostic {
            kind: surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(2433),
            span,
            name: None,
        });
    }
}

/// tsc's `checkFunctionOrConstructorSymbol` across global scripts: a function
/// merged with a non-ambient class is TS2814 at each function and TS2813 at
/// each class. A file holding both reports them itself (the parser's
/// same-file check), so only the other files' declarations are added here.
pub(crate) fn report_cross_file_class_function_merges(parsed_files: &mut [ParsedProgramFile]) {
    struct Member {
        file: usize,
        class: Option<bool>,
        name_span: surge_ts_syntax::TextSpan,
    }
    let mut members: FxHashMap<String, Vec<Member>> = FxHashMap::default();
    for (index, file) in parsed_files.iter().enumerate() {
        if file.is_module || file.file_kind.is_declaration() {
            continue;
        }
        for statement in &file.statements {
            let (name, class, name_span) = match statement {
                ParsedStatement::ClassDeclaration(class) => (&class.name, Some(class.is_declare), class.name_span),
                ParsedStatement::FunctionDeclaration(function) => (&function.name, None, function.name_span),
                _ => continue,
            };
            let Some(name_span) = name_span else { continue };
            members.entry(name.clone()).or_default().push(Member { file: index, class, name_span });
        }
    }
    let mut reports: Vec<(usize, surge_ts_syntax::TextSpan, Option<String>)> = Vec::new();
    for (name, group) in &members {
        let non_ambient_class = |member: &Member| member.class == Some(false);
        let function = |member: &Member| member.class.is_none();
        if !group.iter().any(non_ambient_class) || !group.iter().any(function) {
            continue;
        }
        for member in group {
            let same_file: Vec<&Member> = group.iter().filter(|other| other.file == member.file).collect();
            if same_file.iter().any(|other| non_ambient_class(other)) && same_file.iter().any(|other| function(other)) {
                continue;
            }
            match member.class {
                Some(_) => reports.push((member.file, member.name_span, Some(name.clone()))),
                None => reports.push((member.file, member.name_span, None)),
            }
        }
    }
    for (index, span, class_name) in reports {
        let kind = match class_name {
            Some(_) => surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(2813),
            None => surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(2814),
        };
        parsed_files[index].grammar_diagnostics.push(surge_ts_syntax::ParsedGrammarDiagnostic {
            kind,
            span,
            name: class_name,
        });
    }
}

/// The declaration `statement` adds to a merged symbol table: any statement of
/// a global script, and in a namespace body an exported one — the inner block
/// of a dotted `namespace A.B` among them. A body's other declarations are
/// locals of their own block.
fn merged_member_declaration(statement: &ParsedStatement, in_namespace: bool) -> Option<&ParsedStatement> {
    if !in_namespace {
        return Some(statement);
    }
    match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => Some(declaration.as_ref()),
            _ => None,
        },
        ParsedStatement::NamespaceDeclaration(namespace) if namespace.name.contains('.') => {
            Some(statement)
        }
        _ => None,
    }
}

fn qualified_member_name(prefix: Option<&str>, name: &str) -> String {
    match prefix {
        Some(prefix) => format!("{prefix}.{name}"),
        None => name.to_string(),
    }
}

/// The file each non-ambient class or implemented function of the merged
/// tables is first declared in, by qualified name. Everything in an ambient
/// namespace is ambient.
fn collect_merged_classes_and_functions(
    statements: &[ParsedStatement],
    prefix: Option<&str>,
    index: usize,
    out: &mut FxHashMap<String, usize>,
) {
    for statement in statements {
        let Some(declaration) = merged_member_declaration(statement, prefix.is_some()) else {
            continue;
        };
        let name = match declaration {
            ParsedStatement::ClassDeclaration(class) if !class.is_declare => class.name.as_str(),
            ParsedStatement::FunctionDeclaration(function)
                if function.has_body && !function.is_declare =>
            {
                function.name.as_str()
            }
            ParsedStatement::NamespaceDeclaration(namespace) if !namespace.is_declare => {
                let qualified = qualified_member_name(prefix, namespace.member_name());
                collect_merged_classes_and_functions(
                    &namespace.statements,
                    Some(qualified.as_str()),
                    index,
                    out,
                );
                continue;
            }
            _ => continue,
        };
        out.entry(qualified_member_name(prefix, name)).or_insert(index);
    }
}

/// The name spans of the instantiated non-ambient namespaces in `statements`,
/// nested ones included, whose merged symbol's first class or implemented
/// function is declared in another file.
fn collect_cross_file_namespace_merges(
    statements: &[ParsedStatement],
    prefix: Option<&str>,
    index: usize,
    first_class_or_function: &FxHashMap<String, usize>,
    reports: &mut Vec<(usize, surge_ts_syntax::TextSpan)>,
) {
    for statement in statements {
        let Some(ParsedStatement::NamespaceDeclaration(namespace)) =
            merged_member_declaration(statement, prefix.is_some())
        else {
            continue;
        };
        if namespace.is_declare {
            continue;
        }
        let qualified = qualified_member_name(prefix, namespace.member_name());
        // The inner block of a dotted name carries its outermost segment's span.
        if !namespace.name.contains('.')
            && let Some(span) = namespace.name_span
            && first_class_or_function
                .get(qualified.as_str())
                .is_some_and(|first| *first != index)
            && is_instantiated_namespace(namespace)
        {
            reports.push((index, span));
        }
        collect_cross_file_namespace_merges(
            &namespace.statements,
            Some(qualified.as_str()),
            index,
            first_class_or_function,
            reports,
        );
    }
}
