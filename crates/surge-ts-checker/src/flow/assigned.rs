//! tsc's `markNodeAssignments`: the bindings a flow container assigns
//! anywhere, including in the functions nested in it. `isSymbolAssigned` reads
//! it to decide whether a parameter or `let` is a constant reference.

use std::collections::HashSet;
use std::sync::Arc;

use surge_ts_syntax::{
    ParsedArrowFunctionBody, ParsedBindingName, ParsedClassMember, ParsedExportDeclaration,
    ParsedExpression, ParsedForBindingKind, ParsedFunctionBodyStatement, ParsedStatement,
};

pub(crate) fn assigned_bindings(body: &[ParsedFunctionBodyStatement]) -> HashSet<Arc<str>> {
    let mut names = HashSet::new();
    statements(body, &mut names);
    names
}

/// [`assigned_bindings`] for a file's top-level statements, the container a
/// module-scope binding is declared in.
pub(crate) fn module_assigned_bindings(statements: &[ParsedStatement]) -> HashSet<Arc<str>> {
    let mut names = HashSet::new();
    module_statements(statements, &mut names);
    names
}

fn module_statements(module: &[ParsedStatement], names: &mut HashSet<Arc<str>>) {
    for statement in module {
        match statement {
            ParsedStatement::VariableDeclaration(variable) => {
                if let Some(initializer) = &variable.initializer {
                    expression(initializer, names);
                }
            }
            ParsedStatement::Assignment(assignment) => {
                names.insert(assignment.target_name.as_str().into());
                expression(&assignment.value, names);
            }
            ParsedStatement::MemberAssignment(assignment) => {
                expression(&assignment.target, names);
                expression(&assignment.value, names);
            }
            ParsedStatement::Expression(value) => expression(value, names),
            ParsedStatement::Block(body) => statements(body, names),
            ParsedStatement::If(statement) => {
                expression(&statement.condition, names);
                statements(&statement.then_body, names);
                statements(&statement.else_body, names);
            }
            ParsedStatement::FunctionDeclaration(function) => statements(&function.body, names),
            ParsedStatement::ClassDeclaration(class) => {
                for member in &class.members {
                    match member {
                        ParsedClassMember::Method(method) => statements(&method.body, names),
                        ParsedClassMember::Constructor(constructor) => statements(&constructor.body, names),
                        _ => {}
                    }
                }
            }
            ParsedStatement::NamespaceDeclaration(namespace) => module_statements(&namespace.statements, names),
            ParsedStatement::ExportDeclaration(export) => {
                if let ParsedExportDeclaration::Statement { declaration, .. } = export.as_ref() {
                    module_statements(std::slice::from_ref(declaration.as_ref()), names);
                }
            }
            _ => {}
        }
    }
}

fn statements(body: &[ParsedFunctionBodyStatement], names: &mut HashSet<Arc<str>>) {
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
                if let Some(initializer) = &variable.initializer {
                    expression(initializer, names);
                }
            }
            ParsedFunctionBodyStatement::Return(statement) => {
                if let Some(value) = &statement.expression {
                    expression(value, names);
                }
            }
            ParsedFunctionBodyStatement::Throw(statement) => {
                expression(&statement.expression, names);
            }
            ParsedFunctionBodyStatement::Assignment(assignment) => {
                names.insert(assignment.target_name.as_str().into());
                expression(&assignment.value, names);
            }
            ParsedFunctionBodyStatement::ThisPropertyAssignment(assignment) => {
                expression(&assignment.value, names);
            }
            ParsedFunctionBodyStatement::MemberAssignment(assignment) => {
                expression(&assignment.target, names);
                expression(&assignment.value, names);
            }
            ParsedFunctionBodyStatement::Expression(value) => expression(value, names),
            ParsedFunctionBodyStatement::Block(block) => statements(block, names),
            ParsedFunctionBodyStatement::Function(function) => statements(&function.body, names),
            ParsedFunctionBodyStatement::If(statement) => {
                expression(&statement.condition, names);
                statements(&statement.then_body, names);
                statements(&statement.else_body, names);
            }
            ParsedFunctionBodyStatement::While(statement) => {
                expression(&statement.condition, names);
                statements(&statement.body, names);
            }
            ParsedFunctionBodyStatement::ForOf(statement) => {
                if matches!(statement.binding_kind, ParsedForBindingKind::ExistingBinding) {
                    binding_names(&statement.binding_name, names);
                }
                for (name, _) in &statement.head_names {
                    names.insert(Arc::from(name.as_str()));
                }
                expression(&statement.iterable, names);
                statements(&statement.body, names);
            }
            ParsedFunctionBodyStatement::Switch(statement) => {
                expression(&statement.discriminant, names);
                for case in &statement.cases {
                    if let Some(test) = &case.test {
                        expression(test, names);
                    }
                    statements(&case.consequent, names);
                }
            }
            ParsedFunctionBodyStatement::Try(statement) => {
                statements(&statement.block, names);
                if let Some(handler) = &statement.handler {
                    statements(&handler.body, names);
                }
                statements(&statement.finalizer, names);
            }
            ParsedFunctionBodyStatement::Continue
            | ParsedFunctionBodyStatement::Break
            | ParsedFunctionBodyStatement::BreakOut
            | ParsedFunctionBodyStatement::LabelEnd
            | ParsedFunctionBodyStatement::TypeAlias(_)
            | ParsedFunctionBodyStatement::Interface(_)
            | ParsedFunctionBodyStatement::Class(_) => {}
        }
    }
}

fn expression(value: &ParsedExpression, names: &mut HashSet<Arc<str>>) {
    match value {
        ParsedExpression::Assignment { target_name, .. } => {
            names.insert(target_name.as_str().into());
        }
        ParsedExpression::Update { operand, .. } => {
            if let ParsedExpression::Identifier { name, .. } = operand.as_ref() {
                names.insert(name.as_str().into());
            }
        }
        ParsedExpression::ArrowFunction(function) => match &function.body {
            ParsedArrowFunctionBody::Block(body) => statements(body, names),
            ParsedArrowFunctionBody::Expression(body) => expression(body, names),
        },
        _ => {}
    }
    value.for_each_child(&mut |child| expression(child, names));
}

fn binding_names(binding: &ParsedBindingName, names: &mut HashSet<Arc<str>>) {
    if let ParsedBindingName::Identifier { name, .. } = binding {
        names.insert(name.as_str().into());
    }
}
