//! Grammar errors tsc's checker reports on a tree its parser accepted, for the
//! constructs whose recovery only the parser port reproduces: a variable
//! declaration list with a trailing comma or no declarations at all
//! (`checkGrammarVariableDeclarationList`), and members written beside a
//! mapped type's (`checkGrammarMappedType`, `checkGrammarProperty`), a
//! type parameter list's duplicate names (`checkTypeParameters`), a
//! deferred import's bindings (`checkGrammarImportClause`), and a private
//! name a class uses as a bare expression
//! (`checkGrammarPrivateIdentifierExpression`).

use crate::Diagnostic;
use crate::ast::{NodeId, NodeList};
use crate::kind::Kind;
use crate::messages as diagnostics;
use crate::parser::ParsedFile;
use crate::scanner;

pub(crate) fn checker_grammar_diagnostics(file: &ParsedFile, text: &str) -> Vec<Diagnostic> {
    let mut found = Vec::new();
    let mut stack = vec![(file.root, None, false)];
    while let Some((node, parent, in_class)) = stack.pop() {
        if in_class && parent.is_some_and(|parent| private_name_used_as_expression(file, parent, node)) {
            let (start, end) = crate::binder::error_range_for_node(file, text, node);
            found.push(Diagnostic {
                start,
                end,
                message: diagnostics::Private_identifiers_are_only_allowed_in_class_bodies_and_may_only_be_used_as_part_of_a_class_member_declaration_property_access_or_on_the_left_hand_side_of_an_in_expression,
                args: Vec::new(),
            });
        }
        if let Some(list) = checked_declaration_list(file, node) {
            check_declaration_list(file, text, list, &mut found);
        }
        if let Some(list) = &file.node(node).type_parameters {
            check_duplicate_type_parameters(file, text, list, &mut found);
        }
        for import in checked_imports(file, node) {
            check_deferred_import_clause(file, text, import, &mut found);
        }
        if let Some(first) = member_beside_mapped_type(file, node) {
            let (start, end) = crate::binder::error_range_for_node(file, text, first);
            found.push(Diagnostic {
                start,
                end,
                message: diagnostics::A_mapped_type_may_not_declare_properties_or_methods,
                args: Vec::new(),
            });
        }
        let in_class = in_class || matches!(file.node(node).kind, Kind::ClassDeclaration | Kind::ClassExpression);
        stack.extend(file.children(node).into_iter().map(|child| (child, Some(node), in_class)));
    }
    found.sort_by_key(|diagnostic| diagnostic.start);
    found
}

/// A declaration list the checker checks: a variable statement's, or a `for`,
/// `for…in` or `for…of` head's.
fn checked_declaration_list(file: &ParsedFile, node: NodeId) -> Option<NodeId> {
    let n = file.node(node);
    let list = match n.kind {
        Kind::VariableStatement => n.children.first().copied().flatten(),
        Kind::ForStatement | Kind::ForInStatement | Kind::ForOfStatement => n.initializer,
        _ => None,
    }?;
    (file.node(list).kind == Kind::VariableDeclarationList).then_some(list)
}

fn check_declaration_list(file: &ParsedFile, text: &str, list: NodeId, found: &mut Vec<Diagnostic>) {
    let Some(declarations) = file.node(list).lists.first().and_then(|list| list.as_ref()) else { return };
    if let Some(comma) = trailing_comma(file, text, declarations) {
        found.push(Diagnostic { start: comma, end: comma + 1, message: diagnostics::Trailing_comma_not_allowed, args: Vec::new() });
        return;
    }
    if declarations.nodes.is_empty() {
        found.push(Diagnostic {
            start: declarations.pos,
            end: declarations.end,
            message: diagnostics::Variable_declaration_list_cannot_be_empty,
            args: Vec::new(),
        });
    }
}

/// `checkTypeParameters`: a type parameter named like an earlier one in its
/// list is TS2300 at its name.
fn check_duplicate_type_parameters(file: &ParsedFile, text: &str, list: &NodeList, found: &mut Vec<Diagnostic>) {
    let names: Vec<Option<NodeId>> = list.nodes.iter().map(|&parameter| file.node(parameter).name).collect();
    for (index, name) in names.iter().enumerate() {
        let Some(name) = *name else { continue };
        let spelling = &file.node(name).text;
        if names[..index].iter().flatten().any(|&earlier| file.node(earlier).text == *spelling) {
            let (start, end) = crate::binder::error_range_for_node(file, text, name);
            found.push(Diagnostic { start, end, message: diagnostics::Duplicate_identifier_0, args: vec![spelling.clone()] });
        }
    }
}

/// `NodeList.HasTrailingComma`: the list's range ends with a comma after its
/// last element.
fn trailing_comma(file: &ParsedFile, text: &str, list: &NodeList) -> Option<usize> {
    let last = *list.nodes.last()?;
    let comma = scanner::skip_trivia(text, file.node(last).end);
    (comma < list.end && text.as_bytes().get(comma) == Some(&b',')).then_some(comma)
}

/// A private name the checker checks as an expression where only a member's
/// name, a property access's name, the left of an `in` expression or a
/// `for…in` head (which reports its own error) may hold one.
fn private_name_used_as_expression(file: &ParsedFile, parent: NodeId, node: NodeId) -> bool {
    if file.node(node).kind != Kind::PrivateIdentifier {
        return false;
    }
    let p = file.node(parent);
    let slot = |index: usize| p.children.get(index).copied().flatten() == Some(node);
    match p.kind {
        _ if p.name == Some(node) => false,
        Kind::BindingElement | Kind::QualifiedName => !slot(1),
        Kind::BinaryExpression => !(p.op == Kind::InKeyword && slot(0)),
        Kind::ForInStatement => false,
        _ => true,
    }
}

/// The import declarations whose clause the checker checks
/// (`checkExternalImportOrExportDeclaration`): a source file's, and an
/// ambient module's that import a module by a non-relative name.
fn checked_imports(file: &ParsedFile, node: NodeId) -> Vec<NodeId> {
    let n = file.node(node);
    let (statements, in_ambient_module) = match n.kind {
        Kind::SourceFile => (n.lists.first(), false),
        Kind::ModuleDeclaration if n.name.is_some_and(|name| file.node(name).kind == Kind::StringLiteral) => {
            match n.body.map(|body| file.node(body)) {
                Some(body) if body.kind == Kind::ModuleBlock => (body.lists.first(), true),
                _ => return Vec::new(),
            }
        }
        _ => return Vec::new(),
    };
    let Some(statements) = statements.and_then(|list| list.as_ref()) else { return Vec::new() };
    let imports_module = |statement: NodeId| {
        let statement = file.node(statement);
        statement.kind == Kind::ImportDeclaration
            && statement.children.first().copied().flatten().is_some_and(|specifier| {
                let specifier = file.node(specifier);
                specifier.kind == Kind::StringLiteral
                    && !(in_ambient_module && crate::merge::is_external_module_name_relative(&specifier.text))
            })
    };
    statements.nodes.iter().copied().filter(|&statement| imports_module(statement)).collect()
}

/// `checkGrammarImportClause`: a deferred import binds only a namespace.
fn check_deferred_import_clause(file: &ParsedFile, text: &str, import: NodeId, found: &mut Vec<Diagnostic>) {
    let Some(clause) = file.node(import).import_clause else { return };
    let n = file.node(clause);
    if n.op != Kind::DeferKeyword {
        return;
    }
    let message = if n.name.is_some() {
        diagnostics::Default_imports_are_not_allowed_in_a_deferred_import
    } else if n.children.first().copied().flatten().is_some_and(|bindings| file.node(bindings).kind == Kind::NamedImports) {
        diagnostics::Named_imports_are_not_allowed_in_a_deferred_import
    } else {
        return;
    };
    let (start, end) = crate::binder::error_range_for_node(file, text, clause);
    found.push(Diagnostic { start, end, message, args: Vec::new() });
}

/// The member tsc reports a mapped type's extra members at: the first of a
/// mapped type's own members, or the first member of a type whose property is
/// named like a mapped type's key (`[K in Keys]: T`).
fn member_beside_mapped_type(file: &ParsedFile, node: NodeId) -> Option<NodeId> {
    let members_slot = match file.node(node).kind {
        Kind::MappedType => {
            return file.node(node).lists.first()?.as_ref()?.nodes.first().copied();
        }
        Kind::TypeLiteral => 0,
        Kind::InterfaceDeclaration | Kind::ClassDeclaration | Kind::ClassExpression => 1,
        _ => return None,
    };
    let members = file.node(node).lists.get(members_slot)?.as_ref()?;
    let names_key = |member: NodeId| {
        let member = file.node(member);
        matches!(member.kind, Kind::PropertySignature | Kind::PropertyDeclaration)
            && member.name.is_some_and(|name| {
                let name = file.node(name);
                name.kind == Kind::ComputedPropertyName
                    && name.expression.is_some_and(|expression| {
                        let expression = file.node(expression);
                        expression.kind == Kind::BinaryExpression && expression.op == Kind::InKeyword
                    })
            })
    };
    members.nodes.iter().any(|&member| names_key(member)).then(|| members.nodes[0])
}
