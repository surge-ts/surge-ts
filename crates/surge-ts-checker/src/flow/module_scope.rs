//! Definite assignment over a module-level flow container — a source file or
//! a namespace body. tsc's `checkIdentifier` analyzes a binding's flow from
//! the start of the container that declares it, so a top-level
//! `var x: T;` read before any assignment is TS2454 exactly as it is inside a
//! function. Top-level statements are type-checked one at a time with no flow
//! state, so this pass walks them once more for assignment state alone; it
//! reports only TS2448/TS2454, never a type.

use std::collections::HashMap;
use std::sync::Arc;

use surge_ts_syntax::{
    ParsedDecoratorTarget, ParsedDefaultExportDeclaration, ParsedExportDeclaration, ParsedExpression,
    ParsedFunctionBodyStatement, ParsedStatement, ParsedType, ParsedVariableDeclaration,
    ParsedVariableKind,
};

use super::{
    AssignmentState, FunctionFlowState, analyze_function_body_flow, assignment_value_read,
    is_compound_assignment, check_assignment_target_flow,
    check_expression_flow, collect_hoisted_vars_with, type_assumed_initialized,
};
use crate::context::CheckerContext;

/// How a binding's declared type is known in this container: resolved through
/// the file's module symbols at the top level, or only from its written
/// annotation in a namespace body, whose members are not in that table.
#[derive(Clone, Copy)]
enum DeclaredTypes {
    ModuleSymbols,
    AnnotationOnly,
}

pub(crate) fn check_module_definite_assignment(
    statements: &[ParsedStatement],
    ctx: &mut CheckerContext,
) {
    // Everything a declaration file declares is ambient, which tsc takes as
    // initialized and never in a temporal dead zone.
    if surge_ts_syntax::is_declaration_file_name(&ctx.file_name) {
        return;
    }
    check_container(statements, DeclaredTypes::ModuleSymbols, ctx);
}

fn check_container(statements: &[ParsedStatement], types: DeclaredTypes, ctx: &mut CheckerContext) {
    for statement in statements {
        if let ParsedStatement::NamespaceDeclaration(namespace) = unwrap_export(statement) {
            let saved = super::never_initialized::enter_namespace(&namespace.statements, ctx);
            check_namespace_members(&namespace.statements, ctx);
            check_container(&namespace.statements, DeclaredTypes::AnnotationOnly, ctx);
            ctx.inherited_never_initialized = saved;
        }
    }

    // A `for…of` `var`'s element type is only known through the module table.
    let with_for_of_elements = matches!(types, DeclaredTypes::ModuleSymbols);
    let hoisted: Vec<Arc<str>> = hoisted_module_vars(statements, with_for_of_elements)
        .into_iter()
        .filter(|name| {
            let annotation = var_annotation(statements, name);
            !declared_type_admits_undefined(name, annotation, types, ctx)
        })
        .collect();
    let future_declarations = future_module_declarations(statements);
    if hoisted.is_empty() && future_declarations.is_empty() {
        return;
    }

    let mut flow = FunctionFlowState::new(true);
    flow.shared_statement_index = true;
    // A global script's own `let`s are not mutable locals to tsc
    // (`isMutableLocalVariableDeclaration`).
    flow.script_top_level = matches!(types, DeclaredTypes::ModuleSymbols)
        && !statements.iter().any(|statement| {
            matches!(statement, ParsedStatement::ImportDeclaration(_) | ParsedStatement::ExportDeclaration(_))
        });
    flow.push_scope(future_declarations);
    for name in hoisted {
        flow.hoisted_vars.insert(Arc::clone(&name));
        flow.declare_current(name, AssignmentState::DeclaredUnassigned);
    }
    for (index, statement) in statements.iter().enumerate() {
        check_statement(unwrap_export(statement), index, &mut flow, types, ctx);
    }
}

/// Namespace members produce no value-level checks of their own, so their
/// bodies are walked for definite assignment alone.
fn check_namespace_members(statements: &[ParsedStatement], ctx: &mut CheckerContext) {
    for statement in statements {
        match unwrap_export(statement) {
            ParsedStatement::FunctionDeclaration(function) if !function.is_declare => {
                check_detached_container(
                    &function.type_parameters,
                    &function.parameters,
                    &function.body,
                    ctx,
                );
            }
            ParsedStatement::VariableDeclaration(variable) => {
                if let Some(ParsedExpression::ArrowFunction(arrow)) = &variable.initializer {
                    check_detached_arrow(arrow, ctx);
                }
            }
            ParsedStatement::ClassDeclaration(class) => check_class_member_flow(class, ctx),
            _ => {}
        }
    }
}

fn check_detached_arrow(arrow: &surge_ts_syntax::ParsedArrowFunction, ctx: &mut CheckerContext) {
    match &arrow.body {
        surge_ts_syntax::ParsedArrowFunctionBody::Block(body) => {
            check_detached_container(&arrow.type_parameters, &arrow.parameters, body, ctx);
        }
        surge_ts_syntax::ParsedArrowFunctionBody::Expression(expression) => {
            if let Some(flow) = super::expression_container_flow(&arrow.parameters, ctx) {
                super::check_parameter_default_flow(&arrow.parameters, &flow, ctx);
                let _ = check_expression_flow(expression, arrow.body_span, &flow, 0, ctx);
            }
        }
    }
}

/// The member bodies of a class surge does not type-check — a generic class,
/// or one declared inside a function or namespace — walked for definite
/// assignment alone.
pub(crate) fn check_class_member_flow(
    class: &surge_ts_syntax::ParsedClassDeclaration,
    ctx: &mut CheckerContext,
) {
    if class.is_declare {
        return;
    }
    for member in &class.members {
        match member {
            surge_ts_syntax::ParsedClassMember::Method(method) if !method.is_abstract => {
                let mut type_parameters = class.type_parameters.clone();
                type_parameters.extend(method.type_parameters.iter().cloned());
                check_detached_container(&type_parameters, &method.parameters, &method.body, ctx);
            }
            surge_ts_syntax::ParsedClassMember::Constructor(constructor) => {
                check_detached_container(
                    &class.type_parameters,
                    &constructor.parameters,
                    &constructor.body,
                    ctx,
                );
            }
            surge_ts_syntax::ParsedClassMember::Accessor(accessor) if !accessor.is_abstract => {
                for declaration in &accessor.declarations {
                    if declaration.has_body {
                        check_detached_container(
                            &class.type_parameters,
                            &declaration.parameters,
                            &declaration.body,
                            ctx,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// A function-like body walked for definite assignment alone: its hoisted
/// `var`s and its annotated `let`s are tracked from their written types, and it
/// inherits the never-initialized bindings of the containers around it.
pub(crate) fn check_detached_container(
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    body: &[ParsedFunctionBodyStatement],
    ctx: &mut CheckerContext,
) {
    ctx.push_type_parameter_scope(type_parameters, None);
    let mut flow = FunctionFlowState::new(true);
    flow.detached = true;
    let saved = super::enter_container(type_parameters, parameters, body, &mut flow, ctx);
    super::check_parameter_default_flow(parameters, &flow, ctx);
    let hoisted: Vec<Arc<str>> = collect_hoisted_vars_with(body, false)
        .into_iter()
        .filter(|name| !super::binds_parameter(parameters, name))
        .filter(|name| {
            detached_var_annotation(body, name)
                .is_some_and(|annotation| super::never_initialized::excludes_undefined(annotation, ctx, &[]))
        })
        .collect();
    flow.hoist_vars(hoisted);
    flow.push_scope(HashMap::new());
    let _ = walk_body(body, 0, &mut flow, ctx);
    ctx.inherited_never_initialized = saved;
    ctx.pop_type_parameter_scope();
}

fn detached_var_annotation<'a>(
    body: &'a [ParsedFunctionBodyStatement],
    name: &str,
) -> Option<&'a ParsedType> {
    for statement in body {
        let found = match statement {
            ParsedFunctionBodyStatement::VariableDeclaration(variable)
                if variable.kind == ParsedVariableKind::Var && variable.name == name =>
            {
                return variable.declared_type.as_ref();
            }
            ParsedFunctionBodyStatement::Block(block) => detached_var_annotation(block, name),
            ParsedFunctionBodyStatement::If(if_statement) => detached_var_annotation(&if_statement.then_body, name)
                .or_else(|| detached_var_annotation(&if_statement.else_body, name)),
            ParsedFunctionBodyStatement::While(while_statement) => {
                detached_var_annotation(&while_statement.body, name)
            }
            ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
                detached_var_annotation(&for_of_statement.body, name)
            }
            ParsedFunctionBodyStatement::Switch(switch_statement) => switch_statement
                .cases
                .iter()
                .find_map(|case| detached_var_annotation(&case.consequent, name)),
            ParsedFunctionBodyStatement::Try(try_statement) => {
                detached_var_annotation(&try_statement.block, name)
                    .or_else(|| {
                        try_statement
                            .handler
                            .as_ref()
                            .and_then(|handler| detached_var_annotation(&handler.body, name))
                    })
                    .or_else(|| detached_var_annotation(&try_statement.finalizer, name))
            }
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

fn unwrap_export(statement: &ParsedStatement) -> &ParsedStatement {
    match statement {
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Statement { declaration, .. } => declaration,
            _ => statement,
        },
        _ => statement,
    }
}

fn hoisted_module_vars(statements: &[ParsedStatement], with_for_of_elements: bool) -> Vec<Arc<str>> {
    let mut body = Vec::new();
    for statement in statements {
        match unwrap_export(statement) {
            ParsedStatement::VariableDeclaration(variable) => {
                body.push(ParsedFunctionBodyStatement::VariableDeclaration(variable.clone()))
            }
            ParsedStatement::Block(block) => body.extend(block.iter().cloned()),
            ParsedStatement::If(if_statement) => {
                body.push(ParsedFunctionBodyStatement::If(if_statement.clone()))
            }
            _ => {}
        }
    }
    collect_hoisted_vars_with(&body, with_for_of_elements)
}

fn var_annotation<'a>(statements: &'a [ParsedStatement], name: &str) -> Option<&'a ParsedType> {
    statements.iter().find_map(|statement| match unwrap_export(statement) {
        ParsedStatement::VariableDeclaration(variable)
            if variable.kind == ParsedVariableKind::Var && variable.name == name =>
        {
            variable.declared_type.as_ref()
        }
        _ => None,
    })
}

fn future_module_declarations(statements: &[ParsedStatement]) -> HashMap<Arc<str>, super::FutureDeclaration> {
    let mut declarations = HashMap::new();
    for (index, statement) in statements.iter().enumerate() {
        if let ParsedStatement::VariableDeclaration(variable) = unwrap_export(statement)
            && matches!(variable.kind, ParsedVariableKind::Let | ParsedVariableKind::Const)
            && !variable.is_declare
        {
            declarations
                .entry(variable.name.as_str().into())
                .or_insert(super::FutureDeclaration::of(index, variable));
        }
    }
    declarations
}

fn declared_type_admits_undefined(
    name: &str,
    annotation: Option<&ParsedType>,
    types: DeclaredTypes,
    ctx: &CheckerContext,
) -> bool {
    match types {
        DeclaredTypes::ModuleSymbols => {
            let declared = ctx
                .symbols
                .declared_type(name)
                .cloned()
                .or_else(|| ctx.symbols.get(name).map(|symbol| symbol.ty.clone()));
            // A binding the table cannot type is left alone rather than guessed.
            declared.is_none_or(|declared| type_assumed_initialized(&declared, ctx))
        }
        // Without resolution, only an annotation that is plainly free of
        // `undefined` is analyzed; a named type may alias a union that admits it.
        DeclaredTypes::AnnotationOnly => {
            annotation.is_none_or(|annotation| !super::is_plainly_defined(annotation))
        }
    }
}


fn check_statement(
    statement: &ParsedStatement,
    index: usize,
    flow: &mut FunctionFlowState,
    types: DeclaredTypes,
    ctx: &mut CheckerContext,
) {
    match statement {
        ParsedStatement::VariableDeclaration(variable) => {
            declare_variable(variable, index, flow, types, ctx)
        }
        ParsedStatement::Assignment(assignment) => {
            let _ = check_assignment_target_flow(&assignment.target_name, flow, index, ctx, assignment.target_span);
            let (read, read_span) = assignment_value_read(assignment);
            let _ = check_expression_flow_marking(read, read_span, flow, index, ctx);
            if !is_compound_assignment(&assignment.target_name, assignment.target_span, &assignment.value) {
                flow.mark_assigned(&assignment.target_name);
            }
        }
        ParsedStatement::MemberAssignment(assignment) => {
            let _ = check_expression_flow_marking(
                &assignment.target,
                assignment.target_span,
                flow,
                index,
                ctx,
            );
            let _ =
                check_expression_flow_marking(&assignment.value, assignment.value_span, flow, index, ctx);
        }
        ParsedStatement::Call(call) => {
            let expression = ParsedExpression::Call {
                callee_name: call.callee_name.clone(),
                callee_span: call.callee_span,
                type_arguments: call.type_arguments.clone(),
                arguments: call.arguments.clone(),
            };
            let _ = check_expression_flow_marking(&expression, call.span, flow, index, ctx);
        }
        ParsedStatement::Expression(expression) => {
            let _ = check_expression_flow_marking(expression, None, flow, index, ctx);
            mark_iife_assignments(expression, flow);
        }
        ParsedStatement::If(if_statement) => {
            let statement = ParsedFunctionBodyStatement::If(if_statement.clone());
            let _ = walk_body(std::slice::from_ref(&statement), index, flow, ctx);
        }
        ParsedStatement::Block(block) => {
            let _ = walk_body(block, index, flow, ctx);
        }
        ParsedStatement::ClassDeclaration(class) => walk_class(class, index, flow, ctx),
        ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
            ParsedExportDeclaration::Default { declaration, span } => match declaration {
                ParsedDefaultExportDeclaration::Expression(expression) => {
                    let _ = check_expression_flow_marking(expression, *span, flow, index, ctx);
                }
                ParsedDefaultExportDeclaration::Class(class) => walk_class(class, index, flow, ctx),
                _ => {}
            },
            // `checkExportAssignment` checks the exported value's flow, but a
            // name that `export =` exports is exempt from TS2448
            // (`isBlockScopedNameDeclaredBeforeUse`).
            ParsedExportDeclaration::Equals {
                exported_name,
                exported_name_span,
                ..
            } => {
                if !matches!(
                    flow.read_identifier(exported_name, index),
                    super::FlowReadOutcome::UseBeforeDeclaration { .. }
                ) {
                    let _ = super::report_read_flow(exported_name, *exported_name_span, flow, index, ctx);
                }
            }
            ParsedExportDeclaration::EqualsExpression {
                expression,
                expression_span,
                ..
            } => {
                let _ = check_expression_flow_marking(expression, *expression_span, flow, index, ctx);
            }
            _ => {}
        },
        _ => {}
    }
}

fn declare_variable(
    variable: &ParsedVariableDeclaration,
    index: usize,
    flow: &mut FunctionFlowState,
    types: DeclaredTypes,
    ctx: &mut CheckerContext,
) {
    if let Some(initializer) = &variable.initializer {
        let circular_var_starts = ctx.circular_var_starts.clone();
        let initializing = super::InitializingBinding::for_initializer(
            variable,
            || {
                variable.declared_type.as_ref().map(|annotation| {
                    !declared_type_admits_undefined(&variable.name, Some(annotation), types, ctx)
                })
            },
            &circular_var_starts,
        )
        .map(|binding| flow.begin_initializer([binding]));
        let _ = check_expression_flow_marking(initializer, variable.initializer_span, flow, index, ctx);
        if let Some(mark) = initializing {
            flow.end_initializer(mark);
        }
    }
    // Whether a `var` is tracked at all was settled by its first declaration
    // (see `collect_hoisted_vars_with`); as a later one runs, only an
    // initializer assigns it.
    if variable.kind == ParsedVariableKind::Var {
        if variable.initializer.is_some() {
            flow.mark_assigned(&variable.name);
        }
        return;
    }
    let assigned = variable.initializer.is_some()
        || variable.is_declare
        || variable.has_definite_assertion
        || variable.declared_type.is_none()
        || declared_type_admits_undefined(
            &variable.name,
            variable.declared_type.as_ref(),
            types,
            ctx,
        );
    flow.declare_current(
        variable.name.as_str(),
        if assigned {
            AssignmentState::Assigned
        } else {
            AssignmentState::DeclaredUnassigned
        },
    );
}

/// Walks structured statements for assignment state. A block-local binding
/// shadows the container's in a pushed scope and is taken as assigned: its
/// declared type is not resolved here, and an unresolved guess would report.
fn walk_body(
    body: &[ParsedFunctionBodyStatement],
    index: usize,
    flow: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) -> bool {
    for statement in body {
        if !walk_statement(statement, index, flow, ctx) {
            return false;
        }
    }
    true
}

/// Returns whether control can continue past `statement`.
fn walk_statement(
    statement: &ParsedFunctionBodyStatement,
    index: usize,
    flow: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) -> bool {
    match statement {
        ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
            if let Some(initializer) = &variable.initializer {
                let circular_var_starts = ctx.circular_var_starts.clone();
                let initializing = super::InitializingBinding::for_initializer(
                    variable,
                    || {
                        variable.declared_type.as_ref().map(|annotation| {
                            super::never_initialized::excludes_undefined(annotation, ctx, &[])
                        })
                    },
                    &circular_var_starts,
                )
                .map(|binding| flow.begin_initializer([binding]));
                let _ =
                    check_expression_flow_marking(initializer, variable.initializer_span, flow, index, ctx);
                if let Some(mark) = initializing {
                    flow.end_initializer(mark);
                }
            }
            if variable.kind == ParsedVariableKind::Var {
                if variable.initializer.is_some() {
                    flow.mark_assigned(&variable.name);
                }
            } else if flow.detached
                && variable.initializer.is_none()
                && !variable.is_declare
                && !variable.has_definite_assertion
                && let Some(annotation) = &variable.declared_type
                && super::never_initialized::excludes_undefined(annotation, ctx, &[])
            {
                flow.declare_current(variable.name.as_str(), AssignmentState::DeclaredUnassigned);
                if super::never_initialized::annotation_constraint_substitutes(annotation, ctx, &[]) {
                    flow.constraint_exempt.insert(variable.name.as_str().into());
                }
            } else {
                flow.declare_current(variable.name.as_str(), AssignmentState::Assigned);
            }
            true
        }
        ParsedFunctionBodyStatement::Assignment(assignment) => {
            let _ = check_assignment_target_flow(&assignment.target_name, flow, index, ctx, assignment.target_span);
            let (read, read_span) = assignment_value_read(assignment);
            let _ = check_expression_flow_marking(read, read_span, flow, index, ctx);
            if !is_compound_assignment(&assignment.target_name, assignment.target_span, &assignment.value) {
                flow.mark_assigned(&assignment.target_name);
            }
            true
        }
        ParsedFunctionBodyStatement::MemberAssignment(assignment) => {
            let _ =
                check_expression_flow_marking(&assignment.target, assignment.target_span, flow, index, ctx);
            let _ =
                check_expression_flow_marking(&assignment.value, assignment.value_span, flow, index, ctx);
            true
        }
        ParsedFunctionBodyStatement::ThisPropertyAssignment(assignment) => {
            let _ =
                check_expression_flow_marking(&assignment.value, assignment.value_span, flow, index, ctx);
            true
        }
        ParsedFunctionBodyStatement::Expression(expression) => {
            let _ = check_expression_flow_marking(expression, None, flow, index, ctx);
            mark_iife_assignments(expression, flow);
            true
        }
        ParsedFunctionBodyStatement::Return(statement) => {
            if let Some(expression) = &statement.expression {
                let _ = check_expression_flow_marking(expression, statement.expression_span, flow, index, ctx);
            }
            false
        }
        ParsedFunctionBodyStatement::Throw(statement) => {
            let _ =
                check_expression_flow_marking(&statement.expression, statement.expression_span, flow, index, ctx);
            false
        }
        ParsedFunctionBodyStatement::Continue | ParsedFunctionBodyStatement::Break => false,
        ParsedFunctionBodyStatement::Block(block) => in_block(block, index, flow, ctx),
        ParsedFunctionBodyStatement::If(if_statement) => {
            let _ = check_expression_flow_marking(
                &if_statement.condition,
                if_statement.condition_span,
                flow,
                index,
                ctx,
            );
            let condition = &if_statement.condition;
            let mut then_flow = flow.clone();
            let then_continues = in_edge_block(condition, true, &if_statement.then_body, index, &mut then_flow, ctx);
            let mut else_flow = flow.clone();
            let else_continues = in_edge_block(condition, false, &if_statement.else_body, index, &mut else_flow, ctx);
            join(
                flow,
                [
                    then_continues.then_some(&then_flow),
                    else_continues.then_some(&else_flow),
                ],
            )
        }
        ParsedFunctionBodyStatement::While(while_statement) => {
            let _ = check_expression_flow_marking(
                &while_statement.condition,
                while_statement.condition_span,
                flow,
                index,
                ctx,
            );
            let mut body_flow = flow.clone();
            let body_continues =
                in_edge_block(&while_statement.condition, true, &while_statement.body, index, &mut body_flow, ctx);
            // Only a body that must run adds its assignments to the code after
            // the loop; otherwise the loop may be skipped outright.
            if while_statement.runs_at_least_once
                && body_continues
                && !analyze_function_body_flow(&while_statement.body).guarantees_exit
            {
                adopt(flow, &body_flow);
            }
            true
        }
        ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
            let head = flow.begin_initializer(super::for_head_initializing(for_of_statement));
            let _ = check_expression_flow_marking(
                &for_of_statement.iterable,
                for_of_statement.iterable_span,
                flow,
                index,
                ctx,
            );
            flow.end_initializer(head);
            let mut body_flow = flow.clone();
            if for_of_statement.binding_kind != surge_ts_syntax::ParsedForBindingKind::BlockScoped
                && let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } =
                    &for_of_statement.binding_name
            {
                body_flow.mark_assigned(name);
            }
            for (name, _) in &for_of_statement.head_names {
                body_flow.mark_assigned(name);
            }
            let bound = super::for_head_initializing(for_of_statement)
                .into_iter()
                .map(|binding| binding.name)
                .collect();
            let _ = in_block_binding(&for_of_statement.body, bound, index, &mut body_flow, ctx);
            true
        }
        ParsedFunctionBodyStatement::Switch(switch_statement) => {
            let _ = check_expression_flow_marking(
                &switch_statement.discriminant,
                switch_statement.discriminant_span,
                flow,
                index,
                ctx,
            );
            let has_default = switch_statement.cases.iter().any(|case| case.test.is_none());
            let mut ends = Vec::new();
            for case in &switch_statement.cases {
                if let Some(test) = &case.test {
                    let _ = check_expression_flow_marking(test, case.test_span, flow, index, ctx);
                }
                let mut case_flow = flow.clone();
                let continues = in_block(&case.consequent, index, &mut case_flow, ctx);
                // A `break` ends the case, not the code after the switch.
                let breaks = crate::flow::body_breaks_enclosing_loop(&case.consequent);
                if continues || breaks {
                    ends.push(case_flow);
                }
            }
            let entry = flow.clone();
            let mut joined: Vec<Option<&FunctionFlowState>> = ends.iter().map(Some).collect();
            if !has_default {
                joined.push(Some(&entry));
            }
            join(flow, joined)
        }
        ParsedFunctionBodyStatement::Try(try_statement) => {
            let entry = flow.clone();
            let mut try_flow = flow.clone();
            let try_continues = in_block(&try_statement.block, index, &mut try_flow, ctx);
            let mut ends = vec![try_continues.then_some(try_flow)];
            if let Some(handler) = &try_statement.handler {
                // The handler can be entered before anything in the `try` ran.
                let mut handler_flow = entry.clone();
                let mut bound = Vec::new();
                if let Some(binding) = &handler.binding_name {
                    binding_names(binding, &mut bound);
                }
                let handler_continues = in_block_binding(&handler.body, bound, index, &mut handler_flow, ctx);
                ends.push(handler_continues.then_some(handler_flow));
            }
            let continues = join(flow, ends.iter().map(Option::as_ref));
            if !try_statement.finalizer.is_empty() {
                let mut finalizer_flow = entry;
                let finalizer_continues =
                    in_block(&try_statement.finalizer, index, &mut finalizer_flow, ctx);
                if !finalizer_continues {
                    return false;
                }
                adopt(flow, &finalizer_flow);
            }
            continues
        }
        ParsedFunctionBodyStatement::Class(class) => {
            walk_class(class, index, flow, ctx);
            true
        }
        ParsedFunctionBodyStatement::Function(_)
        | ParsedFunctionBodyStatement::TypeAlias(_)
        | ParsedFunctionBodyStatement::Interface(_) => true,
    }
}

/// A class's `extends` expression and its `static {}` blocks run where the
/// class is declared, as part of the enclosing flow — tsc binds a static block
/// into it rather than as a container of its own. Every other member body is a
/// flow container of its own.
pub(crate) fn walk_class(
    class: &surge_ts_syntax::ParsedClassDeclaration,
    index: usize,
    flow: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    if class.is_declare {
        return;
    }
    for base in &class.extends {
        let root = base.name.split('.').next().unwrap_or(&base.name);
        let read = ParsedExpression::Identifier {
            name: root.to_string(),
            span: base.span,
        };
        let _ = check_expression_flow_marking(&read, base.span, flow, index, ctx);
    }
    let legacy_decorators = ctx.options.experimental_decorators;
    for decorator in &class.decorators {
        if !crate::program::decorator_is_checked(decorator.target, legacy_decorators) {
            continue;
        }
        if decorator.target == ParsedDecoratorTarget::Class {
            let _ = check_expression_flow(&decorator.expression, decorator.span, flow, index, ctx);
        } else {
            check_dead_zone_reads(&decorator.expression, index, flow, ctx);
        }
    }
    for member in &class.members {
        match member {
            surge_ts_syntax::ParsedClassMember::StaticBlock(block) => {
                let _ = in_block(&block.body, index, flow, ctx);
            }
            // A property initializer is a flow container of its own: only a
            // never-initialized outer `let` reports there.
            surge_ts_syntax::ParsedClassMember::Property(property) if !property.is_declare => {
                if let Some(initializer) = &property.initializer
                    && let Some(own_flow) = super::expression_container_flow(&[], ctx)
                {
                    let _ = check_expression_flow(
                        initializer,
                        property.initializer_span,
                        &own_flow,
                        index,
                        ctx,
                    );
                }
            }
            _ => {}
        }
    }
}

/// A member's decorator runs with its class, but tsc checks it from the
/// member's own flow container, where an outer binding is taken as
/// initialized: only a read of one still in its temporal dead zone reports
/// (TS2448).
fn check_dead_zone_reads(
    expression: &ParsedExpression,
    index: usize,
    flow: &FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    match expression {
        ParsedExpression::Identifier { name, span } => {
            let _ = check_assignment_target_flow(name, flow, index, ctx, *span);
        }
        ParsedExpression::Call {
            callee_name,
            callee_span,
            arguments,
            ..
        } => {
            let _ = check_assignment_target_flow(callee_name, flow, index, ctx, *callee_span);
            for argument in arguments {
                check_dead_zone_reads(&argument.expression, index, flow, ctx);
            }
        }
        other => other.for_each_child(&mut |child| check_dead_zone_reads(child, index, flow, ctx)),
    }
}

/// An immediately invoked function is not a flow container of its own in tsc,
/// so what its body assigns is assigned once the call returns. Every
/// assignment in it counts, reached or not — the approximation can only
/// withhold a report.
fn mark_iife_assignments(expression: &ParsedExpression, flow: &mut FunctionFlowState) {
    let ParsedExpression::ExpressionCall { callee, .. } = expression else {
        return;
    };
    let ParsedExpression::ArrowFunction(function) = callee.as_ref() else {
        return;
    };
    let surge_ts_syntax::ParsedArrowFunctionBody::Block(body) = &function.body else {
        return;
    };
    let mut names = Vec::new();
    collect_assigned_names(body, &mut names);
    for name in names {
        flow.mark_assigned(&name);
    }
}

fn collect_assigned_names(body: &[ParsedFunctionBodyStatement], names: &mut Vec<String>) {
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::Assignment(assignment) => {
                names.push(assignment.target_name.clone())
            }
            ParsedFunctionBodyStatement::Block(block) => collect_assigned_names(block, names),
            ParsedFunctionBodyStatement::If(if_statement) => {
                collect_assigned_names(&if_statement.then_body, names);
                collect_assigned_names(&if_statement.else_body, names);
            }
            ParsedFunctionBodyStatement::While(while_statement) => {
                collect_assigned_names(&while_statement.body, names)
            }
            ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
                collect_assigned_names(&for_of_statement.body, names)
            }
            ParsedFunctionBodyStatement::Switch(switch_statement) => {
                for case in &switch_statement.cases {
                    collect_assigned_names(&case.consequent, names);
                }
            }
            ParsedFunctionBodyStatement::Try(try_statement) => {
                collect_assigned_names(&try_statement.block, names);
                if let Some(handler) = &try_statement.handler {
                    collect_assigned_names(&handler.body, names);
                }
                collect_assigned_names(&try_statement.finalizer, names);
            }
            _ => {}
        }
    }
}

fn in_block(
    body: &[ParsedFunctionBodyStatement],
    index: usize,
    flow: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) -> bool {
    in_block_binding(body, Vec::new(), index, flow, ctx)
}

/// [`in_block`] with `bound` — a `for…of` head's or a `catch` clause's
/// bindings — declared in the block's scope. Those, and the functions and
/// classes the block declares, shadow a module binding of the same name that is
/// still in its temporal dead zone.
fn in_block_binding(
    body: &[ParsedFunctionBodyStatement],
    bound: Vec<Arc<str>>,
    index: usize,
    flow: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) -> bool {
    flow.push_scope(HashMap::new());
    for name in bound {
        flow.declare_current(name, AssignmentState::Assigned);
    }
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::Function(function) => {
                flow.declare_current(function.name.as_str(), AssignmentState::Assigned)
            }
            ParsedFunctionBodyStatement::Class(class) => {
                flow.declare_current(class.name.as_str(), AssignmentState::Assigned)
            }
            _ => {}
        }
    }
    let continues = walk_body(body, index, flow, ctx);
    flow.pop_scope();
    continues
}

fn binding_names(binding: &surge_ts_syntax::ParsedBindingName, out: &mut Vec<Arc<str>>) {
    use surge_ts_syntax::ParsedBindingName;
    match binding {
        ParsedBindingName::Identifier { name, .. } => out.push(name.as_str().into()),
        ParsedBindingName::ObjectPattern(pattern) => {
            for element in &pattern.elements {
                binding_names(&element.binding_name, out);
            }
            if let Some(rest) = &pattern.rest {
                binding_names(rest, out);
            }
        }
        ParsedBindingName::ArrayPattern(pattern) => {
            for element in pattern.elements.iter().flatten() {
                binding_names(element, out);
            }
            if let Some(rest) = &pattern.rest {
                binding_names(rest, out);
            }
        }
        ParsedBindingName::Unsupported { .. } => {}
    }
}

/// A block entered on `condition`'s `when` edge, which carries what the edge
/// proves defined; one a literal condition never enters is unreachable, so it
/// reports nothing and reaches no join.
fn in_edge_block(
    condition: &ParsedExpression,
    when: bool,
    body: &[ParsedFunctionBodyStatement],
    index: usize,
    flow: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) -> bool {
    let unreachable = super::condition_never_takes(condition, when);
    super::mark_condition_defined(condition, when, flow, ctx);
    if when {
        for assignment in super::condition_true_assignments(condition) {
            if let ParsedExpression::Assignment { target_name, .. } = assignment {
                flow.mark_assigned(target_name);
            }
        }
    }
    flow.enter_unreachable(unreachable);
    let continues = in_block(body, index, flow, ctx);
    flow.exit_unreachable(unreachable);
    continues && !unreachable
}

/// The container's bindings after edges meet: assigned only where every edge
/// that reaches here assigned them. Returns whether any edge reaches here.
fn join<'a>(
    flow: &mut FunctionFlowState,
    edges: impl IntoIterator<Item = Option<&'a FunctionFlowState>>,
) -> bool {
    let reaching: Vec<&FunctionFlowState> = edges.into_iter().flatten().collect();
    if reaching.is_empty() {
        return false;
    }
    for (name, _) in flow.root_states() {
        let assigned = reaching
            .iter()
            .all(|edge| edge.root_state(&name) == Some(AssignmentState::Assigned));
        flow.set_root_state(
            &name,
            if assigned {
                AssignmentState::Assigned
            } else {
                AssignmentState::DeclaredUnassigned
            },
        );
    }
    true
}

/// Takes every assignment `edge` made to the container's bindings.
fn adopt(flow: &mut FunctionFlowState, edge: &FunctionFlowState) {
    for (name, state) in edge.root_states() {
        if state == AssignmentState::Assigned {
            flow.set_root_state(&name, AssignmentState::Assigned);
        }
    }
}

/// Reads `expression` for definite assignment, then marks what it assigns as
/// it runs (`if ((x = f()))`), so the code after it sees those writes.
fn check_expression_flow_marking(
    expression: &ParsedExpression,
    fallback_span: Option<surge_ts_syntax::TextSpan>,
    flow: &mut FunctionFlowState,
    index: usize,
    ctx: &mut CheckerContext,
) -> super::FlowCheck {
    let result = check_expression_flow(expression, fallback_span, flow, index, ctx);
    super::mark_expression_assignments(expression, flow);
    result
}
