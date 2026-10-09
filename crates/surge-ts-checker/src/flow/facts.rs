//! Function flow-fact collection and return-flow summarization.

use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedExpression, ParsedFunctionBodyStatement, ParsedVariableKind, TextSpan as SyntaxTextSpan,
};

use crate::context::{CheckerContext, convert_span};
use crate::program::{
    record_flow_future_declaration_collection_count, record_flow_identifier_read_count,
    record_flow_return_analysis_walk_count,
};

pub(crate) fn collect_function_flow_facts_from_body(
    body: &[ParsedFunctionBodyStatement],
    facts: &mut FunctionFlowFacts,
) {
    for statement in body {
        collect_function_flow_facts_from_statement(statement, facts);
    }
}

pub(crate) fn collect_function_flow_facts_from_statement(
    statement: &ParsedFunctionBodyStatement,
    facts: &mut FunctionFlowFacts,
) {
    facts.has_branching |= matches!(
        statement,
        ParsedFunctionBodyStatement::If(_)
            | ParsedFunctionBodyStatement::While(_)
            | ParsedFunctionBodyStatement::ForOf(_)
            | ParsedFunctionBodyStatement::Switch(_)
            | ParsedFunctionBodyStatement::Try(_)
    );

    match statement {
        ParsedFunctionBodyStatement::Function(_)
        | ParsedFunctionBodyStatement::TypeAlias(_)
        | ParsedFunctionBodyStatement::Interface(_)
        | ParsedFunctionBodyStatement::Class(_) => {}
        ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
            if matches!(
                variable.kind,
                ParsedVariableKind::Let | ParsedVariableKind::Const
            ) {
                facts.has_let_or_const = true;
                facts.has_future_block_scoped_declarations = true;
                facts.has_uninitialized_let_or_const |= variable.initializer.is_none();
            }
            facts.has_identifier_reads |= variable.initializer.is_some();
            facts.has_assignments |= variable.initializer.is_some();
        }
        ParsedFunctionBodyStatement::Return(return_statement) => {
            facts.has_return_or_throw = true;
            facts.has_identifier_reads |= return_statement.expression.is_some();
        }
        ParsedFunctionBodyStatement::Throw(_) => {
            facts.has_return_or_throw = true;
            facts.has_identifier_reads = true;
        }
        ParsedFunctionBodyStatement::Assignment(_) => {
            facts.has_assignments = true;
            facts.has_identifier_reads = true;
        }
        ParsedFunctionBodyStatement::ThisPropertyAssignment(_)
        | ParsedFunctionBodyStatement::MemberAssignment(_) => {
            facts.has_assignments = true;
            facts.has_identifier_reads = true;
        }
        ParsedFunctionBodyStatement::Expression(_) => {
            facts.has_identifier_reads = true;
        }
        ParsedFunctionBodyStatement::Continue | ParsedFunctionBodyStatement::Break => {}
        ParsedFunctionBodyStatement::Block(block_body) => {
            collect_function_flow_facts_from_body(block_body, facts);
        }
        ParsedFunctionBodyStatement::If(if_statement) => {
            facts.has_identifier_reads = true;
            collect_function_flow_facts_from_body(&if_statement.then_body, facts);
            collect_function_flow_facts_from_body(&if_statement.else_body, facts);
        }
        ParsedFunctionBodyStatement::While(while_statement) => {
            facts.has_identifier_reads = true;
            collect_function_flow_facts_from_body(&while_statement.body, facts);
        }
        ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
            facts.has_identifier_reads = true;
            collect_function_flow_facts_from_body(&for_of_statement.body, facts);
        }
        ParsedFunctionBodyStatement::Switch(switch_statement) => {
            facts.has_identifier_reads = true;
            for case in &switch_statement.cases {
                collect_function_flow_facts_from_body(&case.consequent, facts);
            }
        }
        ParsedFunctionBodyStatement::Try(try_statement) => {
            facts.has_identifier_reads = true;
            collect_function_flow_facts_from_body(&try_statement.block, facts);
            if let Some(handler) = &try_statement.handler {
                collect_function_flow_facts_from_body(&handler.body, facts);
            }
            collect_function_flow_facts_from_body(&try_statement.finalizer, facts);
        }
    }
}

/// The `var` bindings a flow container hoists: every `var` declared anywhere in
/// `body` outside nested functions whose type is known to exclude `undefined` —
/// written, or inferred from an initializer that cannot produce it. tsc analyzes each from the
/// start of its container, so a read ahead of the declaration, or on a path
/// that skipped the initializer, is TS2454. An annotation that plainly admits
/// `undefined` (or is `any`/`unknown`/`void`) is skipped here; one that does so
/// only through an alias is settled when the declaration is reached.
pub(crate) fn collect_hoisted_vars(body: &[ParsedFunctionBodyStatement]) -> Vec<Arc<str>> {
    collect_hoisted_vars_with(body, true)
}

/// As [`collect_hoisted_vars`]; `with_for_of_elements` also hoists a `for…of`
/// `var`, whose element type only a type-checking walk can settle. A `for…in`
/// key is always `string`, so it is hoisted either way.
///
/// A `var` declared more than once is one binding, judged by its first
/// declaration — the symbol's `valueDeclaration`, which is what tsc's
/// `checkIdentifier` reads its ambient flag, `!` and type from. A later
/// `declare var` or `var x!: T` changes nothing about it.
pub(crate) fn collect_hoisted_vars_with(
    body: &[ParsedFunctionBodyStatement],
    with_for_of_elements: bool,
) -> Vec<Arc<str>> {
    struct Hoisting {
        elements: bool,
        names: Vec<Arc<str>>,
        declared: Vec<Arc<str>>,
    }
    impl Hoisting {
        /// Whether `name` is declared here for the first time.
        fn first_declaration(&mut self, name: &str) -> bool {
            if self.declared.iter().any(|declared| declared.as_ref() == name) {
                return false;
            }
            self.declared.push(name.into());
            true
        }
    }
    fn walk(body: &[ParsedFunctionBodyStatement], hoisting: &mut Hoisting) {
        for statement in body {
            match statement {
                ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
                    if variable.kind == ParsedVariableKind::Var
                        && hoisting.first_declaration(&variable.name)
                        && !variable.is_declare
                        && !variable.has_definite_assertion
                        && match &variable.declared_type {
                            Some(declared) => !annotation_admits_undefined(declared),
                            None => variable
                                .initializer
                                .as_ref()
                                .is_some_and(initializer_is_never_undefined),
                        }
                    {
                        hoisting.names.push(variable.name.as_str().into());
                    }
                }
                ParsedFunctionBodyStatement::Block(block) => walk(block, hoisting),
                ParsedFunctionBodyStatement::If(if_statement) => {
                    walk(&if_statement.then_body, hoisting);
                    walk(&if_statement.else_body, hoisting);
                }
                ParsedFunctionBodyStatement::While(while_statement) => {
                    walk(&while_statement.body, hoisting)
                }
                ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
                    if for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::Var
                        && let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } =
                            &for_of_statement.binding_name
                        && hoisting.first_declaration(name)
                        && (hoisting.elements || for_of_statement.keys_only)
                    {
                        hoisting.names.push(name.as_str().into());
                    }
                    walk(&for_of_statement.body, hoisting)
                }
                ParsedFunctionBodyStatement::Switch(switch_statement) => {
                    for case in &switch_statement.cases {
                        walk(&case.consequent, hoisting);
                    }
                }
                ParsedFunctionBodyStatement::Try(try_statement) => {
                    walk(&try_statement.block, hoisting);
                    if let Some(handler) = &try_statement.handler {
                        walk(&handler.body, hoisting);
                    }
                    walk(&try_statement.finalizer, hoisting);
                }
                _ => {}
            }
        }
    }
    let mut hoisting = Hoisting {
        elements: with_for_of_elements,
        names: Vec::new(),
        declared: Vec::new(),
    };
    walk(body, &mut hoisting);
    hoisting.names
}

/// An initializer whose type is never `undefined`, `any` or `unknown`, so the
/// binding it types is analyzed without resolving it.
fn initializer_is_never_undefined(initializer: &ParsedExpression) -> bool {
    matches!(
        initializer,
        ParsedExpression::NumberLiteral(_)
            | ParsedExpression::StringLiteral(_)
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::BigIntLiteral(_)
            | ParsedExpression::TemplateLiteral { .. }
            | ParsedExpression::ObjectLiteral { .. }
            | ParsedExpression::ArrayLiteral { .. }
            | ParsedExpression::New { .. }
    )
}

/// Every `var` declared in `body` outside nested functions, typed or not —
/// including a `for…of`/`for…in` `var` head.
pub(crate) fn collect_var_names(body: &[ParsedFunctionBodyStatement], names: &mut Vec<String>) {
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::VariableDeclaration(variable)
                if variable.kind == ParsedVariableKind::Var =>
            {
                names.push(variable.name.clone())
            }
            ParsedFunctionBodyStatement::Block(block) => collect_var_names(block, names),
            ParsedFunctionBodyStatement::If(if_statement) => {
                collect_var_names(&if_statement.then_body, names);
                collect_var_names(&if_statement.else_body, names);
            }
            ParsedFunctionBodyStatement::While(while_statement) => {
                collect_var_names(&while_statement.body, names)
            }
            ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
                if for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::Var
                    && let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } =
                        &for_of_statement.binding_name
                {
                    names.push(name.clone());
                }
                collect_var_names(&for_of_statement.body, names)
            }
            ParsedFunctionBodyStatement::Switch(switch_statement) => {
                for case in &switch_statement.cases {
                    collect_var_names(&case.consequent, names);
                }
            }
            ParsedFunctionBodyStatement::Try(try_statement) => {
                collect_var_names(&try_statement.block, names);
                if let Some(handler) = &try_statement.handler {
                    collect_var_names(&handler.body, names);
                }
                collect_var_names(&try_statement.finalizer, names);
            }
            _ => {}
        }
    }
}

/// Parameter defaults run first, in the function's own flow: an outer binding
/// they read is judged like a read in the body. They cannot see the body's
/// hoisted `var`s, so this runs before those are hoisted.
pub(crate) fn check_parameter_default_flow(
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    flow_state: &FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    if !flow_state.enabled || flow_state.tracked_local_count == 0 {
        return;
    }
    for parameter in parameters {
        if let Some(initializer) = &parameter.initializer {
            let _ = check_expression_flow(initializer, parameter.initializer_span, flow_state, 0, ctx);
        }
    }
}

/// A `var` redeclaring a parameter is the parameter, already assigned.
pub(crate) fn binds_parameter(
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
    name: &str,
) -> bool {
    parameters.iter().any(|parameter| {
        matches!(&parameter.binding_name,
            surge_ts_syntax::ParsedBindingName::Identifier { name: parameter_name, .. }
                if parameter_name == name)
    })
}

pub(crate) fn annotation_admits_undefined(declared: &surge_ts_syntax::ParsedType) -> bool {
    use surge_ts_syntax::ParsedType;
    match declared {
        ParsedType::Any
        | ParsedType::Unknown
        | ParsedType::UnknownKeyword
        | ParsedType::Void
        | ParsedType::Undefined
        | ParsedType::ErrorType => true,
        ParsedType::Union(members) => members
            .iter()
            .any(|member| !matches!(member, ParsedType::Void) && annotation_admits_undefined(member)),
        _ => false,
    }
}

fn is_always_truthy_condition(condition: &ParsedExpression) -> bool {
    matches!(condition, ParsedExpression::BooleanLiteral(true))
}

fn is_always_falsy_condition(condition: &ParsedExpression) -> bool {
    matches!(condition, ParsedExpression::BooleanLiteral(false))
}

/// Whether `body` contains a `break` that would exit the loop it directly
/// belongs to. Recurses into structured statements that share the loop's break
/// target (`if`/block/`try`) but not into nested loops or `switch`, which
/// capture their own `break`.
pub(crate) fn body_breaks_enclosing_loop(body: &[ParsedFunctionBodyStatement]) -> bool {
    body.iter().any(statement_breaks_enclosing_loop)
}

fn statement_breaks_enclosing_loop(statement: &ParsedFunctionBodyStatement) -> bool {
    match statement {
        ParsedFunctionBodyStatement::Break => true,
        ParsedFunctionBodyStatement::Block(block) => body_breaks_enclosing_loop(block),
        ParsedFunctionBodyStatement::If(if_statement) => {
            body_breaks_enclosing_loop(&if_statement.then_body)
                || body_breaks_enclosing_loop(&if_statement.else_body)
        }
        ParsedFunctionBodyStatement::Try(try_statement) => {
            body_breaks_enclosing_loop(&try_statement.block)
                || try_statement
                    .handler
                    .as_ref()
                    .is_some_and(|handler| body_breaks_enclosing_loop(&handler.body))
                || body_breaks_enclosing_loop(&try_statement.finalizer)
        }
        _ => false,
    }
}

pub(crate) fn summarize_function_body_flow(
    body: &[ParsedFunctionBodyStatement],
) -> ReturnFlowSummary {
    record_flow_return_analysis_walk_count();

    let mut summary = ReturnFlowSummary::default();
    for statement in body {
        let statement_summary = summarize_function_statement_flow(statement);
        // tsc's binder binds a statement no flow reaches without running
        // `bindReturnStatement`, so a `return` there is not an explicit one.
        if !summary.guarantees_exit {
            summary.contains_return |= statement_summary.contains_return;
        }
        summary.contains_return_with_value |= statement_summary.contains_return_with_value;
        summary.contains_throw |= statement_summary.contains_throw;
        summary.guarantees_value_return |= statement_summary.guarantees_value_return;
        summary.guarantees_exit |= statement_summary.guarantees_exit;
    }

    summary
}

pub(crate) fn summarize_function_statement_flow(
    statement: &ParsedFunctionBodyStatement,
) -> ReturnFlowSummary {
    match statement {
        ParsedFunctionBodyStatement::Function(_)
        | ParsedFunctionBodyStatement::TypeAlias(_)
        | ParsedFunctionBodyStatement::Interface(_)
        | ParsedFunctionBodyStatement::Class(_) => ReturnFlowSummary::default(),
        ParsedFunctionBodyStatement::Return(return_statement) => ReturnFlowSummary {
            contains_return: true,
            contains_return_with_value: return_statement.expression.is_some(),
            contains_throw: false,
            guarantees_value_return: return_statement.expression.is_some(),
            guarantees_exit: true,
        },
        ParsedFunctionBodyStatement::Throw(_) => ReturnFlowSummary {
            contains_return: false,
            contains_return_with_value: false,
            contains_throw: true,
            guarantees_value_return: true,
            guarantees_exit: true,
        },
        ParsedFunctionBodyStatement::Continue | ParsedFunctionBodyStatement::Break => {
            ReturnFlowSummary {
                contains_return: false,
                contains_return_with_value: false,
                contains_throw: false,
                guarantees_value_return: false,
                guarantees_exit: true,
            }
        }
        ParsedFunctionBodyStatement::Block(block_body) => summarize_function_body_flow(block_body),
        ParsedFunctionBodyStatement::If(if_statement) => {
            let then_summary = summarize_function_body_flow(&if_statement.then_body);
            let else_summary = summarize_function_body_flow(&if_statement.else_body);

            // tsc's binder makes the branch opposite a literal `true`/`false`
            // condition unreachable, so `if (true) { return 1 }` ends the
            // function and the other branch contributes nothing. It is the
            // *keyword* that folds, not truthiness — `if (1)` still falls
            // through in tsc too.
            if is_always_truthy_condition(&if_statement.condition) {
                return then_summary;
            }
            if is_always_falsy_condition(&if_statement.condition) {
                return else_summary;
            }

            ReturnFlowSummary {
                contains_return: then_summary.contains_return || else_summary.contains_return,
                contains_return_with_value: then_summary.contains_return_with_value
                    || else_summary.contains_return_with_value,
                contains_throw: then_summary.contains_throw || else_summary.contains_throw,
                guarantees_value_return: !if_statement.else_body.is_empty()
                    && then_summary.guarantees_value_return
                    && else_summary.guarantees_value_return,
                guarantees_exit: !if_statement.else_body.is_empty()
                    && then_summary.guarantees_exit
                    && else_summary.guarantees_exit,
            }
        }
        ParsedFunctionBodyStatement::While(while_statement) => {
            let body_summary = summarize_function_body_flow(&while_statement.body);
            // An infinite loop (`while (true)`) with no `break` reaching it never
            // falls through, so control after the loop — including the function's
            // implicit end — is unreachable. Matching tsc's reachability lets the
            // missing-return analysis skip TS7030/TS2366 for these (and narrows
            // unreachable trailing code).
            let never_falls_through = is_always_truthy_condition(&while_statement.condition)
                && !body_breaks_enclosing_loop(&while_statement.body);
            ReturnFlowSummary {
                contains_return: body_summary.contains_return,
                contains_return_with_value: body_summary.contains_return_with_value,
                contains_throw: body_summary.contains_throw,
                guarantees_value_return: never_falls_through,
                guarantees_exit: never_falls_through,
            }
        }
        ParsedFunctionBodyStatement::ForOf(for_of_statement) => {
            let body_summary = summarize_function_body_flow(&for_of_statement.body);
            ReturnFlowSummary {
                contains_return: body_summary.contains_return,
                contains_return_with_value: body_summary.contains_return_with_value,
                contains_throw: body_summary.contains_throw,
                guarantees_value_return: false,
                guarantees_exit: false,
            }
        }
        ParsedFunctionBodyStatement::Switch(switch_statement) => {
            let mut contains_return = false;
            let mut contains_return_with_value = false;
            let mut contains_throw = false;
            // An empty clause falls through to the next one, so only the
            // clauses with a body (and the last, which has nowhere to fall) count.
            let mut guarantees_value_return = switch_statement
                .cases
                .last()
                .is_some_and(|case| !case.consequent.is_empty());

            for case in &switch_statement.cases {
                let case_summary = summarize_function_body_flow(&case.consequent);
                contains_return |= case_summary.contains_return;
                contains_return_with_value |= case_summary.contains_return_with_value;
                contains_throw |= case_summary.contains_throw;
                if !case.consequent.is_empty() {
                    guarantees_value_return &= case_summary.guarantees_value_return;
                }
            }

            // A switch falls through to the following statement unless it is
            // exhaustive (has a `default`), no consequent `break`s out of it, and
            // every clause leaves via `return`/`throw` (empty clauses fall through
            // to the next, so only the last clause must itself terminate). Without
            // this the construct never reports a guaranteed exit, so an exhaustive
            // `switch` whose clauses all `return` looks like it falls through.
            let has_default = switch_statement
                .cases
                .iter()
                .any(|case| case.test.is_none());
            let breaks_out = switch_statement
                .cases
                .iter()
                .any(|case| body_breaks_enclosing_loop(&case.consequent));
            let clauses_terminate = switch_statement.cases.iter().all(|case| {
                case.consequent.is_empty()
                    || summarize_function_body_flow(&case.consequent).guarantees_exit
            });
            let last_clause_terminates = switch_statement.cases.last().is_some_and(|case| {
                !case.consequent.is_empty()
                    && summarize_function_body_flow(&case.consequent).guarantees_exit
            });
            // tsc's `isExhaustiveSwitchStatement` stands in for a `default`, but
            // only on evidence: a discriminant surge could not type proves
            // nothing, and the end point stays reachable.
            let exhaustive = has_default || is_known_exhaustive(switch_statement.span);
            let guarantees_exit =
                exhaustive && !breaks_out && clauses_terminate && last_clause_terminates;

            // Without a `default`, a switch whose clauses all return still falls
            // through unless its cases cover the discriminant's type — which the
            // checker decides and records while checking the body.
            let guarantees_value_return = guarantees_value_return
                && (has_default || !is_known_non_exhaustive(switch_statement.span));

            ReturnFlowSummary {
                contains_return,
                contains_return_with_value,
                contains_throw,
                guarantees_value_return,
                guarantees_exit,
            }
        }
        ParsedFunctionBodyStatement::Try(try_statement) => {
            let block_summary = summarize_function_body_flow(&try_statement.block);
            let handler_summary = try_statement
                .handler
                .as_ref()
                .map(|handler| summarize_function_body_flow(&handler.body));
            let finalizer_summary = summarize_function_body_flow(&try_statement.finalizer);

            let handler_guarantees = handler_summary
                .as_ref()
                .is_none_or(|summary| summary.guarantees_value_return);

            // The try completes normally only if its block completes normally
            // (and, when present, its handler does too on the throwing path). A
            // `return`/`throw` in the finalizer overrides everything. Without this
            // the construct never reports a guaranteed exit, so `try { return }
            // catch { throw }` looks like it falls through.
            let body_and_handler_exit = block_summary.guarantees_exit
                && handler_summary
                    .as_ref()
                    .is_none_or(|summary| summary.guarantees_exit);

            ReturnFlowSummary {
                contains_return: block_summary.contains_return
                    || handler_summary
                        .as_ref()
                        .is_some_and(|summary| summary.contains_return)
                    || finalizer_summary.contains_return,
                contains_return_with_value: block_summary.contains_return_with_value
                    || handler_summary
                        .as_ref()
                        .is_some_and(|summary| summary.contains_return_with_value)
                    || finalizer_summary.contains_return_with_value,
                contains_throw: block_summary.contains_throw
                    || handler_summary
                        .as_ref()
                        .is_some_and(|summary| summary.contains_throw)
                    || finalizer_summary.contains_throw,
                guarantees_value_return: handler_guarantees
                    && (block_summary.guarantees_value_return || block_summary.contains_throw),
                guarantees_exit: body_and_handler_exit || finalizer_summary.guarantees_exit,
            }
        }
        // tsc ends the flow at a call statement whose callee returns `never`,
        // as at a `throw` (`isReachableFlowNode` on a `FlowCall`).
        ParsedFunctionBodyStatement::Expression(expression)
            if call_statement_key(expression).is_some_and(is_known_never_call) =>
        {
            ReturnFlowSummary {
                contains_return: false,
                contains_return_with_value: false,
                contains_throw: true,
                guarantees_value_return: true,
                guarantees_exit: true,
            }
        }
        ParsedFunctionBodyStatement::VariableDeclaration(_)
        | ParsedFunctionBodyStatement::Assignment(_)
        | ParsedFunctionBodyStatement::ThisPropertyAssignment(_)
        | ParsedFunctionBodyStatement::MemberAssignment(_)
        | ParsedFunctionBodyStatement::Expression(_) => ReturnFlowSummary::default(),
    }
}

/// The key a call statement is known by: its callee (or called member) span.
pub(crate) fn call_statement_key(expression: &surge_ts_syntax::ParsedExpression) -> Option<(usize, usize)> {
    let span = match expression {
        surge_ts_syntax::ParsedExpression::Call { callee_span, .. } => *callee_span,
        surge_ts_syntax::ParsedExpression::PropertyCall { property_span, .. } => *property_span,
        _ => None,
    }?;
    Some((span.start, span.end))
}

/// A block-scoped binding declared later in its scope: the statement that
/// declares it, and whether a read before it is also TS2454 — not for a
/// binding element, nor when the annotation is a type tsc assumes initialized
/// (`checkIdentifier`'s `isBindingElement` and `AnyOrUnknown | Void`, or the
/// `errorType` a circular annotation gets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FutureDeclaration {
    pub(crate) index: usize,
    pub(crate) unassigned: bool,
}

impl FutureDeclaration {
    pub(crate) fn of(index: usize, variable: &surge_ts_syntax::ParsedVariableDeclaration) -> Self {
        let assumed_initialized = variable.from_binding_pattern
            || matches!(
                variable.declared_type,
                Some(
                    surge_ts_syntax::ParsedType::Any
                        | surge_ts_syntax::ParsedType::UnknownKeyword
                        | surge_ts_syntax::ParsedType::Void
                        | surge_ts_syntax::ParsedType::ErrorType
                )
            );
        Self {
            index,
            unassigned: !assumed_initialized,
        }
    }
}

pub(crate) fn collect_future_block_scoped_declarations(
    body: &[ParsedFunctionBodyStatement],
) -> HashMap<Arc<str>, FutureDeclaration> {
    let mut declarations = HashMap::new();

    for (index, statement) in body.iter().enumerate() {
        if let ParsedFunctionBodyStatement::VariableDeclaration(variable) = statement {
            if matches!(
                variable.kind,
                ParsedVariableKind::Let | ParsedVariableKind::Const
            ) {
                declarations
                    .entry(variable.name.clone().into())
                    .or_insert(FutureDeclaration::of(index, variable));
            }
        }
    }

    record_flow_future_declaration_collection_count(declarations.len());
    declarations
}

pub(crate) fn report_read_flow(
    name: &str,
    span: Option<SyntaxTextSpan>,
    flow_state: &FunctionFlowState,
    statement_index: usize,
    ctx: &mut CheckerContext,
) -> FlowCheck {
    report_read_flow_positioned(name, span, flow_state, statement_index, ctx, false)
}

/// As [`report_read_flow`]; `substituting_position` is a read tsc gives a
/// type parameter's union constraint instead (see
/// [`FunctionFlowState::constraint_exempt`]).
pub(crate) fn report_read_flow_positioned(
    name: &str,
    span: Option<SyntaxTextSpan>,
    flow_state: &FunctionFlowState,
    statement_index: usize,
    ctx: &mut CheckerContext,
    substituting_position: bool,
) -> FlowCheck {
    record_flow_identifier_read_count();
    if substituting_position && flow_state.constraint_exempt.contains(name) {
        return FlowCheck::Clear;
    }
    if !flow_state.enabled || flow_state.tracked_local_count == 0 {
        return FlowCheck::Clear;
    }
    if flow_state.assumed_initialized_in_member_key(name) {
        return FlowCheck::Clear;
    }

    match flow_state.read_identifier(name, statement_index) {
        FlowReadOutcome::Unresolved | FlowReadOutcome::Declared(AssignmentState::Assigned) => {
            FlowCheck::Clear
        }
        // Without `strictNullChecks` tsc assumes every variable initialized
        // (`assumeInitialized`), so nothing is used before being assigned.
        FlowReadOutcome::Declared(AssignmentState::DeclaredUnassigned)
            if !surge_ts_types::strict_null_checks() =>
        {
            FlowCheck::Clear
        }
        FlowReadOutcome::Declared(AssignmentState::DeclaredUnassigned)
            if flow_state.reads_as_defined(name) =>
        {
            FlowCheck::Clear
        }
        FlowReadOutcome::Declared(AssignmentState::DeclaredUnassigned) => {
            let mut diagnostic = Diagnostic::ts2454(name, ctx.file_name.clone());
            if let Some(span) = span {
                diagnostic = diagnostic.with_span(convert_span(span));
            }

            ctx.push(diagnostic);
            record_unassigned_read(span, &ctx.file_name);
            // tsc's `checkIdentifier` goes on with the declared type "to reduce
            // follow-on errors", so the expression around the read is checked.
            FlowCheck::Clear
        }
        FlowReadOutcome::UseBeforeDeclaration {
            unassigned,
            circular_at,
        } => {
            // tsc's `checkResolvedBlockScopedVariable`: an enum read before its
            // declaration is TS2450, and a `const enum` (inlined, with no
            // binding to be early of) is not reported outside isolatedModules.
            if let Some(is_const) = flow_state.enum_object(name) {
                if is_const {
                    return FlowCheck::Clear;
                }
                let mut diagnostic = Diagnostic::ts2450(name, ctx.file_name.clone());
                if let Some(span) = span {
                    diagnostic = diagnostic.with_span(convert_span(span));
                }
                ctx.push(diagnostic);
                return FlowCheck::Blocked;
            }
            // A block-scoped (`let`/`const`) variable read before its declaration
            // is necessarily in its temporal dead zone, so it is also definitely
            // unassigned: tsc reports TS2454 beside TS2448 unless the binding's
            // type assumes it initialized.
            let mut diagnostic = Diagnostic::ts2448(name, ctx.file_name.clone());
            if let Some(span) = span {
                diagnostic = diagnostic.with_span(convert_span(span));
            }
            ctx.push(diagnostic);

            if unassigned && surge_ts_types::strict_null_checks() {
                let mut diagnostic = Diagnostic::ts2454(name, ctx.file_name.clone());
                if let Some(span) = span {
                    diagnostic = diagnostic.with_span(convert_span(span));
                }
                ctx.push(diagnostic);
                record_unassigned_read(span, &ctx.file_name);
            }

            if let Some(declaration_span) = circular_at
                && ctx.options.no_implicit_any
            {
                ctx.push(
                    Diagnostic::ts7022(name, ctx.file_name.clone())
                        .with_span(convert_span(declaration_span)),
                );
            }

            FlowCheck::Blocked
        }
    }
}

thread_local! {
    static NON_EXHAUSTIVE_SWITCHES: std::cell::RefCell<Vec<(usize, usize)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static EXHAUSTIVE_SWITCHES: std::cell::RefCell<Vec<(usize, usize)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static NEVER_CALLS: std::cell::RefCell<Vec<(usize, usize)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn is_known_never_call(key: (usize, usize)) -> bool {
    NEVER_CALLS.with(|keys| keys.borrow().contains(&key))
}

/// Runs `summarize` with the call statements known to return `never`, which
/// the syntactic summary cannot tell from any other call.
pub(crate) fn with_never_calls<R>(never_calls: &[(usize, usize)], summarize: impl FnOnce() -> R) -> R {
    let previous = NEVER_CALLS.with(|installed| std::mem::replace(&mut *installed.borrow_mut(), never_calls.to_vec()));
    let result = summarize();
    NEVER_CALLS.with(|installed| *installed.borrow_mut() = previous);
    result
}

fn is_known_exhaustive(span: Option<surge_ts_syntax::TextSpan>) -> bool {
    let Some(span) = span else {
        return false;
    };
    EXHAUSTIVE_SWITCHES.with(|spans| spans.borrow().contains(&(span.start, span.end)))
}

fn is_known_non_exhaustive(span: Option<surge_ts_syntax::TextSpan>) -> bool {
    let Some(span) = span else {
        return false;
    };
    NON_EXHAUSTIVE_SWITCHES.with(|spans| spans.borrow().contains(&(span.start, span.end)))
}

/// Runs `summarize` with what checking learned about the `default`-less
/// switches: which do not cover their discriminant and which do. Scoped to the
/// call, so every other flow summary keeps the syntactic answer.
pub(crate) fn with_non_exhaustive_switches<R>(
    non_exhaustive: &[(usize, usize)],
    exhaustive: &[(usize, usize)],
    summarize: impl FnOnce() -> R,
) -> R {
    let previous_non_exhaustive = NON_EXHAUSTIVE_SWITCHES
        .with(|installed| std::mem::replace(&mut *installed.borrow_mut(), non_exhaustive.to_vec()));
    let previous_exhaustive = EXHAUSTIVE_SWITCHES
        .with(|installed| std::mem::replace(&mut *installed.borrow_mut(), exhaustive.to_vec()));
    let result = summarize();
    NON_EXHAUSTIVE_SWITCHES.with(|installed| *installed.borrow_mut() = previous_non_exhaustive);
    EXHAUSTIVE_SWITCHES.with(|installed| *installed.borrow_mut() = previous_exhaustive);
    result
}
