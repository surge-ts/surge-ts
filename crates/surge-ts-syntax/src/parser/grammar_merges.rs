//! Declaration-merging consistency over one statement list: what tsc checks
//! against a merged symbol's `Declarations` when they all live in one
//! container of one file. Cross-file merging (global scripts) is not seen
//! here.

use oxc_ast::ast::{
    BindingPattern, ClassElement, Declaration, ExportDefaultDeclarationKind, MethodDefinitionKind,
    PropertyDefinitionType, PropertyKey, Statement, TSAccessibility, TSModuleDeclaration,
    TSModuleDeclarationBody, TSModuleDeclarationName, TSSignature,
};
use oxc_span::{GetSpan, Span};

use super::spans::text_span_from_oxc_span;
use crate::{ParsedGrammarDiagnostic, ParsedGrammarDiagnosticKind as Kind};

pub(crate) fn check_merged_declarations(
    statements: &[Statement<'_>],
    ambient: bool,
    source_text: &str,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    let declarations = collect(statements, ambient);
    let mut seen: Vec<&str> = Vec::new();
    for declaration in &declarations {
        if seen.contains(&declaration.name) {
            continue;
        }
        seen.push(declaration.name);
        let group: Vec<&Merged<'_>> = declarations
            .iter()
            .filter(|other| other.name == declaration.name)
            .collect();
        check_enum_first_member_initializers(&group, out);
        if group.len() < 2 {
            // One class merges its properties with its parameter properties.
            if matches!(declaration.kind, MergedKind::Class(_)) {
                check_identical_property_modifiers(&group, source_text, out);
            }
            continue;
        }
        check_namespace_placement(&group, out);
        check_class_function_merge(&group, out);
        check_identical_property_modifiers(&group, source_text, out);
        let namespaces: Vec<&Merged<'_>> = group
            .iter()
            .copied()
            .filter(|declaration| matches!(declaration.kind, MergedKind::Namespace { .. }))
            .collect();
        if namespaces.len() > 1 {
            check_namespace_member_merges(&namespaces, out);
        }
    }
    // Every `export default` declaration also declares the module's `default`.
    let defaults: Vec<(&str, Merged<'_>)> = statements
        .iter()
        .filter_map(|statement| {
            let Statement::ExportDefaultDeclaration(export) = statement else {
                return None;
            };
            let (id, span, declare, kind) = match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    (class.id.as_ref()?, class.span, class.declare, MergedKind::Class(class))
                }
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => (
                    function.id.as_ref()?,
                    function.span,
                    function.declare,
                    MergedKind::Function { has_body: function.body.is_some() },
                ),
                _ => return None,
            };
            Some((id.name.as_str(), Merged { name: "default", name_span: id.span, span, ambient: ambient || declare, kind }))
        })
        .collect();
    // Same-named ones already merged under their own name above.
    let one_local_name = defaults.windows(2).all(|pair| pair[0].0 == pair[1].0);
    let defaults: Vec<Merged<'_>> = defaults.into_iter().map(|(_, merged)| merged).collect();
    if defaults.len() > 1 && !one_local_name {
        check_class_function_merge(&defaults.iter().collect::<Vec<_>>(), out);
    }
}

struct Merged<'a> {
    name: &'a str,
    name_span: Span,
    span: Span,
    ambient: bool,
    kind: MergedKind<'a>,
}

enum MergedKind<'a> {
    Class(&'a oxc_ast::ast::Class<'a>),
    Function { has_body: bool },
    Interface(&'a oxc_ast::ast::TSInterfaceDeclaration<'a>),
    Enum(&'a oxc_ast::ast::TSEnumDeclaration<'a>),
    Namespace {
        instantiated: bool,
        declaration: &'a TSModuleDeclaration<'a>,
    },
    Other,
}

fn collect<'a>(statements: &'a [Statement<'a>], ambient: bool) -> Vec<Merged<'a>> {
    let mut declarations = Vec::new();
    for statement in statements {
        let declaration = match statement {
            Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
            Statement::ExportDefaultDeclaration(export) => {
                match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        if let Some(id) = &class.id {
                            declarations.push(Merged {
                                name: id.name.as_str(),
                                name_span: id.span,
                                span: class.span,
                                ambient: ambient || class.declare,
                                kind: MergedKind::Class(class),
                            });
                        }
                    }
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        if let Some(id) = &function.id {
                            declarations.push(Merged {
                                name: id.name.as_str(),
                                name_span: id.span,
                                span: function.span,
                                ambient: ambient || function.declare,
                                kind: MergedKind::Function {
                                    has_body: function.body.is_some(),
                                },
                            });
                        }
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(interface) => {
                        declarations.push(Merged {
                            name: interface.id.name.as_str(),
                            name_span: interface.id.span,
                            span: interface.span,
                            ambient: ambient || interface.declare,
                            kind: MergedKind::Interface(interface),
                        });
                    }
                    _ => {}
                }
                continue;
            }
            other => other.as_declaration(),
        };
        let Some(declaration) = declaration else {
            continue;
        };
        collect_declaration(declaration, ambient, &mut declarations);
    }
    declarations
}

fn collect_declaration<'a>(declaration: &'a Declaration<'a>, ambient: bool, declarations: &mut Vec<Merged<'a>>) {
    let merged = match declaration {
        Declaration::ClassDeclaration(class) => class.id.as_ref().map(|id| Merged {
            name: id.name.as_str(),
            name_span: id.span,
            span: class.span,
            ambient: ambient || class.declare,
            kind: MergedKind::Class(class),
        }),
        Declaration::FunctionDeclaration(function) => function.id.as_ref().map(|id| Merged {
            name: id.name.as_str(),
            name_span: id.span,
            span: function.span,
            ambient: ambient || function.declare,
            kind: MergedKind::Function {
                has_body: function.body.is_some(),
            },
        }),
        Declaration::TSInterfaceDeclaration(interface) => Some(Merged {
            name: interface.id.name.as_str(),
            name_span: interface.id.span,
            span: interface.span,
            ambient: ambient || interface.declare,
            kind: MergedKind::Interface(interface),
        }),
        Declaration::TSEnumDeclaration(declaration) => Some(Merged {
            name: declaration.id.name.as_str(),
            name_span: declaration.id.span,
            span: declaration.span,
            ambient: ambient || declaration.declare,
            kind: MergedKind::Enum(declaration),
        }),
        Declaration::TSModuleDeclaration(declaration) => merged_namespace(declaration, ambient),
        Declaration::TSTypeAliasDeclaration(alias) => Some(Merged {
            name: alias.id.name.as_str(),
            name_span: alias.id.span,
            span: alias.span,
            ambient,
            kind: MergedKind::Other,
        }),
        Declaration::VariableDeclaration(declaration) => {
            for declarator in &declaration.declarations {
                if let Some(id) = declarator.id.get_binding_identifier() {
                    declarations.push(Merged {
                        name: id.name.as_str(),
                        name_span: id.span,
                        span: declarator.span,
                        ambient: ambient || declaration.declare,
                        kind: MergedKind::Other,
                    });
                }
            }
            None
        }
        Declaration::TSImportEqualsDeclaration(declaration) => Some(Merged {
            name: declaration.id.name.as_str(),
            name_span: declaration.id.span,
            span: declaration.span,
            ambient,
            kind: MergedKind::Other,
        }),
        Declaration::TSGlobalDeclaration(_) => None,
    };
    declarations.extend(merged);
}

fn merged_namespace<'a>(declaration: &'a TSModuleDeclaration<'a>, ambient: bool) -> Option<Merged<'a>> {
    match &declaration.id {
        TSModuleDeclarationName::Identifier(id) => Some(Merged {
            name: id.name.as_str(),
            name_span: id.span,
            span: declaration.span,
            ambient: ambient || declaration.declare,
            kind: MergedKind::Namespace {
                instantiated: super::reachability::is_instantiated_module(declaration),
                declaration,
            },
        }),
        TSModuleDeclarationName::StringLiteral(_) => None,
    }
}

/// The namespaces of one name merge their *exported* members across blocks
/// (`namespace A.B.C {}` beside `namespace A { export namespace B { export
/// class C {} } }`), which no single block's check sees. Only a group drawn
/// from two blocks or more is checked here; one block's own groups are its
/// own check's.
fn check_namespace_member_merges<'a>(namespaces: &[&Merged<'a>], out: &mut Vec<ParsedGrammarDiagnostic>) {
    let mut members: Vec<(usize, Merged<'a>)> = Vec::new();
    for (block, namespace) in namespaces.iter().enumerate() {
        let MergedKind::Namespace { declaration, .. } = namespace.kind else {
            continue;
        };
        let mut exported = Vec::new();
        match &declaration.body {
            // `namespace A.B {}`: `B` is an exported member of `A`.
            Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => {
                exported.extend(merged_namespace(inner, namespace.ambient));
            }
            Some(TSModuleDeclarationBody::TSModuleBlock(body)) => {
                for statement in &body.body {
                    if let Statement::ExportNamedDeclaration(export) = statement
                        && let Some(declaration) = &export.declaration
                    {
                        collect_declaration(declaration, namespace.ambient, &mut exported);
                    }
                }
            }
            None => {}
        }
        members.extend(exported.into_iter().map(|member| (block, member)));
    }
    let mut seen: Vec<&str> = Vec::new();
    for (_, member) in &members {
        if seen.contains(&member.name) {
            continue;
        }
        seen.push(member.name);
        let group: Vec<(usize, &Merged<'a>)> = members
            .iter()
            .filter(|(_, other)| other.name == member.name)
            .map(|(block, other)| (*block, other))
            .collect();
        if group.iter().all(|(block, _)| *block == group[0].0) {
            continue;
        }
        let group: Vec<&Merged<'a>> = group.into_iter().map(|(_, member)| member).collect();
        check_namespace_placement(&group, out);
        check_class_function_merge(&group, out);
        let inner: Vec<&Merged<'a>> = group
            .iter()
            .copied()
            .filter(|member| matches!(member.kind, MergedKind::Namespace { .. }))
            .collect();
        if inner.len() > 1 {
            check_namespace_member_merges(&inner, out);
        }
    }
}

/// tsc's `checkEnumDeclaration`: across the declarations of one enum, only
/// the first may leave its first member without an initializer — TS2432 at
/// every later such member.
fn check_enum_first_member_initializers(group: &[&Merged<'_>], out: &mut Vec<ParsedGrammarDiagnostic>) {
    let mut seen_missing_initializer = false;
    for declaration in group {
        let MergedKind::Enum(declaration) = declaration.kind else {
            continue;
        };
        let Some(first) = declaration.body.members.first() else {
            continue;
        };
        if first.initializer.is_some() {
            continue;
        }
        if seen_missing_initializer {
            push(out, 2432, first.id.span(), None);
        } else {
            seen_missing_initializer = true;
        }
    }
}

/// tsc's `checkModuleDeclaration`: a non-ambient instantiated namespace that
/// merges with a class or a function with a body must follow the first such
/// declaration — TS2434 at the namespace name when it comes earlier.
fn check_namespace_placement(group: &[&Merged<'_>], out: &mut Vec<ParsedGrammarDiagnostic>) {
    let first_class_or_function = group.iter().find(|declaration| {
        !declaration.ambient
            && matches!(
                declaration.kind,
                MergedKind::Class(_) | MergedKind::Function { has_body: true }
            )
    });
    let Some(first) = first_class_or_function else {
        return;
    };
    for declaration in group {
        if declaration.ambient {
            continue;
        }
        let MergedKind::Namespace { instantiated: true, .. } = declaration.kind else {
            continue;
        };
        if declaration.span.start < first.span.start {
            push(out, 2434, declaration.name_span, None);
        }
    }
}

/// tsc's `checkFunctionOrConstructorSymbol`: a function may merge only with
/// an ambient class — TS2813 at each class, TS2814 at each function.
fn check_class_function_merge(group: &[&Merged<'_>], out: &mut Vec<ParsedGrammarDiagnostic>) {
    let has_non_ambient_class = group
        .iter()
        .any(|declaration| !declaration.ambient && matches!(declaration.kind, MergedKind::Class(_)));
    let has_function = group
        .iter()
        .any(|declaration| matches!(declaration.kind, MergedKind::Function { .. }));
    if !has_non_ambient_class || !has_function {
        return;
    }
    for declaration in group {
        match declaration.kind {
            MergedKind::Class(_) => push(out, 2813, declaration.name_span, Some(declaration.name)),
            MergedKind::Function { .. } => push(out, 2814, declaration.name_span, None),
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PropertyFlags {
    optional: bool,
    readonly: bool,
    private: bool,
    protected: bool,
    is_abstract: bool,
}

/// tsc's `areDeclarationFlagsIdentical` over the property declarations that
/// merge into one symbol: the instance properties, parameter properties
/// included, of the classes and interfaces of one name. The first declaration
/// is the symbol's value declaration; it and every declaration differing from
/// it report TS2687.
fn check_identical_property_modifiers(
    group: &[&Merged<'_>],
    source_text: &str,
    out: &mut Vec<ParsedGrammarDiagnostic>,
) {
    let mut properties: Vec<(String, Span, PropertyFlags)> = Vec::new();
    for declaration in group {
        match declaration.kind {
            MergedKind::Class(class) => {
                for element in &class.body.body {
                    if let ClassElement::MethodDefinition(method) = element
                        && method.kind == MethodDefinitionKind::Constructor
                    {
                        for parameter in &method.value.params.items {
                            if !(parameter.accessibility.is_some() || parameter.readonly || parameter.r#override) {
                                continue;
                            }
                            let BindingPattern::BindingIdentifier(identifier) = &parameter.pattern else {
                                continue;
                            };
                            properties.push((
                                identifier.name.to_string(),
                                identifier.span,
                                PropertyFlags {
                                    optional: parameter.optional,
                                    readonly: parameter.readonly,
                                    private: parameter.accessibility == Some(TSAccessibility::Private),
                                    protected: parameter.accessibility == Some(TSAccessibility::Protected),
                                    is_abstract: false,
                                },
                            ));
                        }
                        continue;
                    }
                    let ClassElement::PropertyDefinition(property) = element else {
                        continue;
                    };
                    if property.r#static || property.computed {
                        continue;
                    }
                    let Some(name) = literal_property_name(&property.key) else {
                        continue;
                    };
                    properties.push((
                        name,
                        property.key.span(),
                        PropertyFlags {
                            optional: property.optional,
                            readonly: property.readonly,
                            private: property.accessibility == Some(TSAccessibility::Private),
                            protected: property.accessibility == Some(TSAccessibility::Protected),
                            is_abstract: property.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition,
                        },
                    ));
                }
            }
            MergedKind::Interface(interface) => {
                for member in &interface.body.body {
                    let TSSignature::TSPropertySignature(property) = member else {
                        continue;
                    };
                    if property.computed {
                        continue;
                    }
                    let Some(name) = literal_property_name(&property.key) else {
                        continue;
                    };
                    properties.push((
                        name,
                        property.key.span(),
                        PropertyFlags {
                            optional: property.optional,
                            readonly: property.readonly,
                            private: false,
                            protected: false,
                            is_abstract: false,
                        },
                    ));
                }
            }
            _ => {}
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for (name, _, _) in &properties {
        if seen.contains(&name.as_str()) {
            continue;
        }
        seen.push(name);
        let same: Vec<&(String, Span, PropertyFlags)> =
            properties.iter().filter(|(other, _, _)| other == name).collect();
        let Some((_, _, first_flags)) = same.first() else {
            continue;
        };
        if !same.iter().any(|(_, _, flags)| flags != first_flags) {
            continue;
        }
        for (index, (_, span, flags)) in same.iter().enumerate() {
            if index == 0 || flags != first_flags {
                let written = source_text
                    .get(span.start as usize..span.end as usize)
                    .unwrap_or(name);
                push(out, 2687, *span, Some(written));
            }
        }
    }
}

fn literal_property_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        PropertyKey::NumericLiteral(literal) => Some(literal.value.to_string()),
        _ => None,
    }
}

fn push(out: &mut Vec<ParsedGrammarDiagnostic>, code: u32, span: Span, name: Option<&str>) {
    out.push(ParsedGrammarDiagnostic {
        kind: Kind::Ts(code),
        span: text_span_from_oxc_span(span),
        name: name.map(str::to_string),
    });
}

/// [`check_enum_first_member_initializers`] for an enum a namespace exports
/// from several of its blocks: the blocks merge into one namespace, so their
/// exported enums are one enum.
pub(crate) fn check_namespace_enum_initializers(statements: &[Statement<'_>], out: &mut Vec<ParsedGrammarDiagnostic>) {
    fn walk<'a>(
        statements: &'a [Statement<'a>],
        path: &str,
        blocks: &mut usize,
        found: &mut Vec<(String, usize, &'a oxc_ast::ast::TSEnumDeclaration<'a>)>,
    ) {
        for statement in statements {
            let declaration = match statement {
                Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                other => other.as_declaration(),
            };
            let Some(Declaration::TSModuleDeclaration(module)) = declaration else {
                continue;
            };
            module_body(module, path, blocks, found);
        }
    }
    fn module_body<'a>(
        module: &'a oxc_ast::ast::TSModuleDeclaration<'a>,
        path: &str,
        blocks: &mut usize,
        found: &mut Vec<(String, usize, &'a oxc_ast::ast::TSEnumDeclaration<'a>)>,
    ) {
        let TSModuleDeclarationName::Identifier(id) = &module.id else {
            return;
        };
        let inner_path = format!("{path}{}.", id.name);
        match &module.body {
            Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleDeclaration(inner)) => {
                module_body(inner, &inner_path, blocks, found);
            }
            Some(oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block)) => {
                *blocks += 1;
                let block_id = *blocks;
                for statement in &block.body {
                    if let Statement::ExportNamedDeclaration(export) = statement
                        && let Some(Declaration::TSEnumDeclaration(declaration)) = &export.declaration
                    {
                        found.push((format!("{inner_path}{}", declaration.id.name), block_id, declaration));
                    }
                }
                walk(&block.body, &inner_path, blocks, found);
            }
            None => {}
        }
    }
    let mut found = Vec::new();
    let mut blocks = 0;
    walk(statements, "", &mut blocks, &mut found);
    let mut seen: Vec<&str> = Vec::new();
    for (key, _, _) in &found {
        if seen.contains(&key.as_str()) {
            continue;
        }
        seen.push(key);
        let group: Vec<&(String, usize, &oxc_ast::ast::TSEnumDeclaration<'_>)> =
            found.iter().filter(|(other, _, _)| other == key).collect();
        if group.iter().all(|(_, block, _)| *block == group[0].1) {
            continue;
        }
        let mut seen_missing_initializer = false;
        for (_, _, declaration) in &group {
            let Some(first) = declaration.body.members.first() else {
                continue;
            };
            if first.initializer.is_some() {
                continue;
            }
            if seen_missing_initializer {
                push(out, 2432, first.id.span(), None);
            } else {
                seen_missing_initializer = true;
            }
        }
    }
}
