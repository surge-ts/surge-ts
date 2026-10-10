use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedExpression, ParsedForOfStatement, ParsedFunctionBodyStatement, ParsedIfStatement,
    ParsedSwitchStatement, ParsedTryStatement, ParsedWhileStatement,
};
use surge_ts_types::{Type, TypeCopyReason, union_type, with_type_copy_reason};

use super::super::narrowing::narrow_reference_non_null_in_scope;
use super::super::{
    check_function_body, evaluate_condition_expression_with_truthy_guards, insert_binding_name,
    narrow_discriminant_in_scope, narrow_truthy_guarded_identifiers, visible_symbols,
};
use super::{
    adopt_branch_assignments, apply_condition_true_assignment_types, apply_expression_assignments,
    body_ends_in_never_call, branch_assigned_names, branch_assignment_types, deep_assigned_names,
    join_branch_assignments, join_branch_edges, join_branch_narrowings, join_branch_pair, loop_join_names,
    nested_assigned_names,
    restore_branch_entry,
    mark_condition_true_assignments, narrow_aliased_guard_after_exit,
    narrow_condition_and_aliases_in_scope, narrow_tuple_destructure_siblings, prime_loop_mutations,
    release_loop_mutations, resolved_alias_condition, rewrite_discriminant_aliases,
    widen_assigned_bindings, widen_loop_assigned_bindings,
};
use crate::checks::expr::evaluate_expression;
use crate::context::CheckerContext;
use crate::context::convert_span;
use crate::flow::{
    FlowCheck, FunctionFlowState, analyze_function_body_flow, check_expression_flow,
    merge_branch_deltas,
};
use crate::infer::{InferredExpression, map_parsed_type};
use crate::symbols::ScopeStack;

/// TS2774: a condition that tests a function for truthiness, and never calls or
/// otherwise mentions it where the test holds, is always true — tsc's
/// `checkTestingKnownTruthyType`. Only a type that is certainly callable counts:
/// a union (an optional method, `F | undefined`) has no call signature of its own.
pub(crate) fn report_unreferenced_callable_conditions(
    tests: &[surge_ts_syntax::ParsedTruthinessTest],
    symbols: &crate::symbols::SymbolTable,
    ctx: &mut CheckerContext,
) {
    if !surge_ts_types::strict_null_checks() {
        return;
    }
    for test in tests {
        let InferredExpression::Known(ty) =
            crate::infer::infer_expression(&test.expression, symbols, ctx)
        else {
            continue;
        };
        let callable = match ty.peeled() {
            Type::Function(_) => true,
            Type::Object(object) => object.call_signature().is_some(),
            _ => false,
        };
        if callable && let Some(span) = test.span {
            ctx.push(Diagnostic::ts2774(ctx.file_name.clone()).with_span(convert_span(span)));
        }
    }
}

/// A condition whose assignments have run by the time it is tested reads as
/// their targets (`if ((x = f()))` narrows `x`). A condition that *is* an
/// assignment of an assignment-free value is kept as written: the narrowers
/// read it by its value first and then by its target's truthiness (tsc's
/// `narrowTypeByBinaryExpression`), so `if (a &&= b)` proves `b` truthy too.
fn assignment_condition_reads(condition: &ParsedExpression) -> Option<ParsedExpression> {
    fn is_plain_assignment(condition: &ParsedExpression) -> bool {
        match condition {
            ParsedExpression::Assignment { value, .. } => !value.contains_assignment(),
            ParsedExpression::Unary {
                operator: surge_ts_syntax::ParsedUnaryOperator::Not,
                operand,
                ..
            } => is_plain_assignment(operand),
            _ => false,
        }
    }
    (condition.contains_assignment() && !is_plain_assignment(condition))
        .then(|| condition.with_assignments_as_reads())
}

pub(crate) fn check_function_if_statement(
    if_statement: ParsedIfStatement,
    statement_index: usize,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    report_unreferenced_callable_conditions(
        &if_statement.unreferenced_truthiness_tests,
        &visible_symbols(scopes),
        ctx,
    );
    // `if (ok)` where `ok` is a `const` alias of a condition narrows by the
    // condition the alias was written as — and, in each branch below, by `ok`
    // itself as well: `const perf = inBrowser && source; if (perf)` leaves
    // `perf` truthy, whatever it says about `inBrowser` and `source`.
    // An assignment in the condition has run by the time it is tested:
    // `if ((x = f()))` narrows `x`.
    let narrowing_source = assignment_condition_reads(&if_statement.condition);
    let narrowing_condition = narrowing_source.as_ref().unwrap_or(&if_statement.condition);
    let alias_condition = resolved_alias_condition(narrowing_condition, scopes, flow_state);
    let base_condition: &ParsedExpression =
        alias_condition.as_deref().unwrap_or(narrowing_condition);
    let rewritten_condition = rewrite_discriminant_aliases(base_condition, flow_state);
    // The identifier an alias condition replaced: it is narrowed by the test
    // itself, not only by the expression it stands for.
    let alias_source = alias_condition.as_ref().map(|_| narrowing_condition);

    let then_flow = analyze_function_body_flow(&if_statement.then_body);
    let then_guarantees_value_return = then_flow.guarantees_value_return;
    // The code after `if (cond) <body>` sees `!cond` whenever the then-branch
    // cannot fall through — that includes `continue`/`break` (which only
    // `guarantees_exit` reports), not just a value `return`. Gating narrowing on
    // either keeps the old return-based behavior and adds early-`continue` guards.
    let then_diverts_control = then_guarantees_value_return
        || then_flow.guarantees_exit
        || body_ends_in_never_call(&if_statement.then_body, scopes, ctx);
    let has_else_body = !if_statement.else_body.is_empty();

    let else_flow_diverts = has_else_body && {
        let else_flow = analyze_function_body_flow(&if_statement.else_body);
        else_flow.guarantees_value_return
            || else_flow.guarantees_exit
            || body_ends_in_never_call(&if_statement.else_body, scopes, ctx)
    };
    // tsc's post-`if` flow node joins the branches that can complete. When only
    // one can, the code after the `if` sees exactly what that branch left: its
    // condition narrowing, the fall-through of an `else if` nested in it, and
    // whatever it assigned. The no-else form below is the same rule with an
    // empty surviving branch.
    // tsc folds only the `true`/`false` keywords out of the flow graph, and the
    // branch such a condition never takes does not reach the join.
    let condition_always_true = matches!(if_statement.condition, ParsedExpression::BooleanLiteral(true));
    let condition_always_false = matches!(if_statement.condition, ParsedExpression::BooleanLiteral(false));
    let only_then_completes =
        has_else_body && (else_flow_diverts || condition_always_true) && !then_diverts_control;
    let only_else_completes =
        has_else_body && (then_diverts_control || condition_always_false) && !else_flow_diverts;
    let both_complete = has_else_body
        && !then_diverts_control
        && !else_flow_diverts
        && !condition_always_true
        && !condition_always_false;
    let mut surviving_narrowings = Vec::new();
    let mut joinable_assignments = Vec::new();
    if !has_else_body && !then_diverts_control {
        nested_assigned_names(&if_statement.then_body, &mut joinable_assignments);
    } else if has_else_body && !then_diverts_control && !else_flow_diverts {
        // Both edges reach the join, so the binding is the union of what each
        // branch left it as — the fall-through edge the no-else form uses does
        // not exist here.
        nested_assigned_names(&if_statement.then_body, &mut joinable_assignments);
        nested_assigned_names(&if_statement.else_body, &mut joinable_assignments);
    }

    let mut then_written = Vec::new();
    nested_assigned_names(&if_statement.then_body, &mut then_written);

    let flow_active = flow_state.tracked_local_count() > 0;
    let condition_blocked = if flow_active {
        check_expression_flow(
            &if_statement.condition,
            if_statement.condition_span,
            flow_state,
            statement_index,
            ctx,
        )
    } else {
        FlowCheck::Clear
    };

    if !condition_blocked.is_blocked() {
        let visible_symbols = visible_symbols(scopes);
        let _ = evaluate_condition_expression_with_truthy_guards(
            &if_statement.condition,
            if_statement.condition_span,
            &visible_symbols,
            ctx,
        );
    }
    apply_expression_assignments(&if_statement.condition, scopes, flow_state, ctx);

    if flow_active {
        let mut branch_deltas = Vec::new();
        let then_entry = branch_assignment_types(&then_written, scopes);
        scopes.push_child();
        apply_condition_true_assignment_types(&if_statement.condition, scopes, ctx);
        narrow_condition_and_aliases_in_scope(
            base_condition,
            rewritten_condition.as_ref(),
            alias_source,
            scopes,
            true,
            ctx,
        );
        if alias_condition.is_some() {
            narrow_discriminant_in_scope(&if_statement.condition, scopes, true, ctx);
        }
        flow_state.begin_branch_capture();
        mark_condition_true_assignments(&if_statement.condition, flow_state);
        crate::flow::mark_condition_defined(&if_statement.condition, true, flow_state, ctx);
        let then_unreachable = crate::flow::condition_never_takes(&if_statement.condition, true);
        flow_state.enter_unreachable(then_unreachable);
        check_function_body(
            if_statement.then_body,
            with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
            scopes,
            flow_state,
            ctx,
        );
        flow_state.exit_unreachable(then_unreachable);
        let mut then_delta = flow_state.finish_branch_capture();
        then_delta.continues = !then_diverts_control && !condition_always_false;
        let then_assignment_types = branch_assignment_types(&joinable_assignments, scopes);
        if only_then_completes {
            surviving_narrowings = scopes.current_frame_narrowings();
        }
        let then_narrowings = both_complete.then(|| scopes.current_frame_narrowings());
        scopes.pop_child();
        restore_branch_entry(&then_entry, scopes);
        if !has_else_body {
            join_branch_assignments(&then_assignment_types, base_condition, scopes, ctx);
        }
        branch_deltas.push(then_delta);

        if has_else_body {
            let else_flow = analyze_function_body_flow(&if_statement.else_body);
            let else_diverts_control =
                else_flow.guarantees_value_return || else_flow.guarantees_exit;
            scopes.push_child();
            narrow_condition_and_aliases_in_scope(
                base_condition,
                rewritten_condition.as_ref(),
                alias_source,
                scopes,
                false,
                ctx,
            );
            if alias_condition.is_some() {
                narrow_discriminant_in_scope(&if_statement.condition, scopes, false, ctx);
            }
            flow_state.begin_branch_capture();
            crate::flow::mark_condition_defined(&if_statement.condition, false, flow_state, ctx);
            let else_unreachable =
                crate::flow::condition_never_takes(&if_statement.condition, false);
            flow_state.enter_unreachable(else_unreachable);
            check_function_body(if_statement.else_body, return_type, scopes, flow_state, ctx);
            flow_state.exit_unreachable(else_unreachable);
            let mut else_delta = flow_state.finish_branch_capture();
            else_delta.continues = !else_diverts_control && !condition_always_true;
            let else_assignment_types = branch_assignment_types(&joinable_assignments, scopes);
            if only_else_completes {
                surviving_narrowings = scopes.current_frame_narrowings();
            }
            let else_narrowings = then_narrowings.as_ref().map(|_| scopes.current_frame_narrowings());
            scopes.pop_child();
            join_branch_pair(&then_assignment_types, &else_assignment_types, scopes);
            if let (Some(then_narrowings), Some(else_narrowings)) = (&then_narrowings, &else_narrowings) {
                join_branch_narrowings(then_narrowings, else_narrowings, &joinable_assignments, scopes);
            }
            branch_deltas.push(else_delta);
        }

        scopes.adopt_narrowings(std::mem::take(&mut surviving_narrowings));

        if !has_else_body && then_diverts_control {
            narrow_truthy_guarded_identifiers(base_condition, scopes);
            if let Some(alias_source) = alias_source {
                narrow_truthy_guarded_identifiers(alias_source, scopes);
            }
            narrow_condition_and_aliases_in_scope(
                base_condition,
                rewritten_condition.as_ref(),
                alias_source,
                scopes,
                false,
                ctx,
            );
            if alias_condition.is_some() {
                narrow_discriminant_in_scope(&if_statement.condition, scopes, false, ctx);
            }
            narrow_aliased_guard_after_exit(&if_statement.condition, scopes, flow_state);
        }

        // Without an `else`, the condition's false edge is the other way to
        // the join, carrying what that edge proves defined.
        if !has_else_body {
            flow_state.begin_branch_capture();
            crate::flow::mark_condition_defined(&if_statement.condition, false, flow_state, ctx);
            let mut fallthrough_delta = flow_state.finish_branch_capture();
            fallthrough_delta.continues = !condition_always_true;
            branch_deltas.push(fallthrough_delta);
        }
        merge_branch_deltas(flow_state, &branch_deltas, false);
    } else {
        let then_entry = branch_assignment_types(&then_written, scopes);
        scopes.push_child();
        narrow_condition_and_aliases_in_scope(
            base_condition,
            rewritten_condition.as_ref(),
            alias_source,
            scopes,
            true,
            ctx,
        );
        if alias_condition.is_some() {
            narrow_discriminant_in_scope(&if_statement.condition, scopes, true, ctx);
        }
        check_function_body(
            if_statement.then_body,
            with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
            scopes,
            flow_state,
            ctx,
        );
        let then_assignment_types = branch_assignment_types(&joinable_assignments, scopes);
        if only_then_completes {
            surviving_narrowings = scopes.current_frame_narrowings();
        }
        let then_narrowings = both_complete.then(|| scopes.current_frame_narrowings());
        scopes.pop_child();
        restore_branch_entry(&then_entry, scopes);
        if !has_else_body {
            join_branch_assignments(&then_assignment_types, base_condition, scopes, ctx);
        }

        if has_else_body {
            scopes.push_child();
            narrow_condition_and_aliases_in_scope(
                base_condition,
                rewritten_condition.as_ref(),
                alias_source,
                scopes,
                false,
                ctx,
            );
            if alias_condition.is_some() {
                narrow_discriminant_in_scope(&if_statement.condition, scopes, false, ctx);
            }
            check_function_body(if_statement.else_body, return_type, scopes, flow_state, ctx);
            let else_assignment_types = branch_assignment_types(&joinable_assignments, scopes);
            if only_else_completes {
                surviving_narrowings = scopes.current_frame_narrowings();
            }
            let else_narrowings = then_narrowings.as_ref().map(|_| scopes.current_frame_narrowings());
            scopes.pop_child();
            join_branch_pair(&then_assignment_types, &else_assignment_types, scopes);
            if let (Some(then_narrowings), Some(else_narrowings)) = (&then_narrowings, &else_narrowings) {
                join_branch_narrowings(then_narrowings, else_narrowings, &joinable_assignments, scopes);
            }
        }

        scopes.adopt_narrowings(std::mem::take(&mut surviving_narrowings));

        if !has_else_body && then_diverts_control {
            narrow_truthy_guarded_identifiers(base_condition, scopes);
            if let Some(alias_source) = alias_source {
                narrow_truthy_guarded_identifiers(alias_source, scopes);
            }
            narrow_condition_and_aliases_in_scope(
                base_condition,
                rewritten_condition.as_ref(),
                alias_source,
                scopes,
                false,
                ctx,
            );
            if alias_condition.is_some() {
                narrow_discriminant_in_scope(&if_statement.condition, scopes, false, ctx);
            }
            narrow_aliased_guard_after_exit(&if_statement.condition, scopes, flow_state);
        }
    }
}

/// The edges `continue` statements take to the innermost loop's update
/// clause. A loop with no update clause is a barrier: its own `continue`s go
/// nowhere this tracks.
struct ContinueTarget;

type ContinueEdges = Option<(Vec<String>, Vec<Vec<(String, Type)>>)>;

thread_local! {
    static CONTINUE_TARGETS: std::cell::RefCell<Vec<ContinueEdges>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl ContinueTarget {
    fn enter() -> Self {
        CONTINUE_TARGETS.with(|targets| targets.borrow_mut().push(None));
        ContinueTarget
    }
}

impl Drop for ContinueTarget {
    fn drop(&mut self) {
        CONTINUE_TARGETS.with(|targets| {
            targets.borrow_mut().pop();
        });
    }
}

/// A `continue` reaching the innermost loop's update clause carries what the
/// bindings the loop body assigns hold here.
pub(crate) fn record_continue_edge(scopes: &ScopeStack) {
    CONTINUE_TARGETS.with(|targets| {
        if let Some(Some((names, edges))) = targets.borrow_mut().last_mut() {
            edges.push(branch_assignment_types(names, scopes));
        }
    });
}

/// The body of a lowered `for` runs into its update clause from its end and
/// from every `continue` (tsc binds `continue` to the incrementor label).
fn check_loop_body(
    mut body: Vec<ParsedFunctionBodyStatement>,
    has_update: bool,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    if !has_update || body.is_empty() {
        check_function_body(body, return_type, scopes, flow_state, ctx);
        return;
    }
    let update = body.split_off(1);
    let names = loop_join_names(&body);
    let body_flow = analyze_function_body_flow(&body);
    CONTINUE_TARGETS.with(|targets| {
        if let Some(top) = targets.borrow_mut().last_mut() {
            *top = Some((names.clone(), Vec::new()));
        }
    });
    check_function_body(body, return_type, scopes, flow_state, ctx);
    let mut edges = CONTINUE_TARGETS.with(|targets| {
        targets
            .borrow_mut()
            .last_mut()
            .and_then(Option::take)
            .map(|(_, edges)| edges)
            .unwrap_or_default()
    });
    if !edges.is_empty() {
        if !body_flow.guarantees_exit && !body_flow.guarantees_value_return {
            edges.push(branch_assignment_types(&names, scopes));
        }
        join_branch_edges(&edges, scopes);
    }
    check_function_body(update, return_type, scopes, flow_state, ctx);
}

pub(crate) fn check_function_while_statement(
    while_statement: ParsedWhileStatement,
    statement_index: usize,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let ParsedWhileStatement {
        condition,
        condition_span,
        body,
        runs_at_least_once,
        has_update,
    } = while_statement;
    let _barrier = ContinueTarget::enter();

    // A lowered `do … while (c)` runs its body first, so the body is checked
    // with the enclosing flow state (its assignments stand afterwards) and the
    // condition is checked last, where it may read what the body assigned.
    // tsc types the code after a loop from every edge into it: the body's end
    // (the assignments it made) and, unless the body always runs, the state
    // before the first iteration.
    let assigned = loop_join_names(&body);
    // With nothing in the body breaking out, the loop is left only by its
    // condition failing, so the code after it sees the condition's false edge.
    let exits_by_condition_only = !crate::flow::body_breaks_enclosing_loop(&body);
    // Otherwise each `if (guard) break;` is an edge out too; what every edge
    // proves defined is defined after the loop.
    let exit_defined = (!exits_by_condition_only && !runs_at_least_once && flow_state.tracked_local_count() > 0)
        .then(|| loop_exit_defined_names(&condition, &body, ctx))
        .flatten();
    widen_loop_assigned_bindings(&body, return_type, scopes, flow_state, ctx);
    if runs_at_least_once {
        scopes.push_child();
        let pending_mutations = prime_loop_mutations(&body, scopes, ctx);
        check_loop_body(body, has_update, return_type, scopes, flow_state, ctx);
        release_loop_mutations(pending_mutations, scopes, ctx);
        let body_types = branch_assignment_types(&assigned, scopes);
        scopes.pop_child();
        adopt_branch_assignments(&body_types, scopes);
        check_while_condition(
            &condition,
            condition_span,
            statement_index,
            scopes,
            flow_state,
            ctx,
        );
        if exits_by_condition_only {
            narrow_by_loop_exit(&condition, scopes, flow_state, ctx);
        }
        return;
    }

    check_while_condition(
        &condition,
        condition_span,
        statement_index,
        scopes,
        flow_state,
        ctx,
    );

    let entry_types = branch_assignment_types(&assigned, scopes);
    scopes.push_child();
    // The condition is tested before every iteration, so the body starts where
    // it held, whatever the previous iteration assigned.
    // An assignment in the condition has run by the time it is tested:
    // `while ((m = re.exec(s)))` narrows `m`.
    let narrowing_source = assignment_condition_reads(&condition);
    let narrowing_condition = narrowing_source.as_ref().unwrap_or(&condition);
    let alias_condition = resolved_alias_condition(narrowing_condition, scopes, flow_state);
    let base_condition: &ParsedExpression =
        alias_condition.as_deref().unwrap_or(narrowing_condition);
    let rewritten_condition = rewrite_discriminant_aliases(base_condition, flow_state);
    // The identifier an alias condition replaced: it is narrowed by the test
    // itself, not only by the expression it stands for.
    let alias_source = alias_condition.as_ref().map(|_| narrowing_condition);
    apply_condition_true_assignment_types(&condition, scopes, ctx);
    narrow_condition_and_aliases_in_scope(
        base_condition,
        rewritten_condition.as_ref(),
        alias_source,
        scopes,
        true,
        ctx,
    );
    if alias_condition.is_some() {
        narrow_discriminant_in_scope(&condition, scopes, true, ctx);
    }
    let pending_mutations = prime_loop_mutations(&body, scopes, ctx);
    if flow_state.tracked_local_count() > 0 {
        flow_state.begin_branch_capture();
        mark_condition_true_assignments(&condition, flow_state);
        check_loop_body(body, has_update, return_type, scopes, flow_state, ctx);
        let _ = flow_state.finish_branch_capture();
    } else {
        check_loop_body(body, has_update, return_type, scopes, flow_state, ctx);
    }
    release_loop_mutations(pending_mutations, scopes, ctx);
    let body_types = branch_assignment_types(&assigned, scopes);
    scopes.pop_child();
    join_branch_pair(&entry_types, &body_types, scopes);
    if exits_by_condition_only {
        narrow_by_loop_exit(&condition, scopes, flow_state, ctx);
    }
    for name in exit_defined.into_iter().flatten() {
        flow_state.mark_assigned(&name);
    }
}

/// The names defined on every edge out of a `while` whose body breaks only
/// through top-level `if (guard) break;`: the condition's false edge, and each
/// break, taken where both the loop condition and its guard held. `None` for
/// any other break.
fn loop_exit_defined_names(
    condition: &ParsedExpression,
    body: &[ParsedFunctionBodyStatement],
    ctx: &CheckerContext,
) -> Option<Vec<String>> {
    let predicate = |callee: &str| crate::flow::predicate_parameter(callee, ctx);
    let names = |expression: &ParsedExpression, when: bool| -> Vec<String> {
        crate::flow::condition_defined_names(expression, when, &predicate)
            .into_iter()
            .map(str::to_string)
            .collect()
    };
    let mut defined = names(condition, false);
    let held = names(condition, true);
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::If(if_statement)
                if matches!(if_statement.then_body.as_slice(), [ParsedFunctionBodyStatement::Break])
                    && !crate::flow::body_breaks_enclosing_loop(&if_statement.else_body) =>
            {
                let at_break = names(&if_statement.condition, true);
                defined.retain(|name| held.contains(name) || at_break.contains(name));
            }
            other if crate::flow::body_breaks_enclosing_loop(std::slice::from_ref(other)) => return None,
            _ => {}
        }
    }
    Some(defined)
}

/// The loop's exit is its condition's false edge (`bindWhileStatement`'s
/// `postLoopLabel`): narrowed by it, and definitely assigned by it.
fn narrow_by_loop_exit(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let tested = assignment_condition_reads(condition);
    let tested = tested.as_ref().unwrap_or(condition);
    narrow_discriminant_in_scope(tested, scopes, false, ctx);
    if flow_state.tracked_local_count() > 0 {
        crate::flow::mark_condition_defined(tested, false, flow_state, ctx);
    }
}

fn check_while_condition(
    condition: &surge_ts_syntax::ParsedExpression,
    condition_span: Option<surge_ts_syntax::TextSpan>,
    statement_index: usize,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let condition_blocked = if flow_state.tracked_local_count() > 0 {
        check_expression_flow(condition, condition_span, flow_state, statement_index, ctx)
    } else {
        FlowCheck::Clear
    };

    if !condition_blocked.is_blocked() {
        let visible_symbols = visible_symbols(scopes);
        let _ = evaluate_condition_expression_with_truthy_guards(
            condition,
            condition_span,
            &visible_symbols,
            ctx,
        );
    }
    apply_expression_assignments(condition, scopes, flow_state, ctx);
}

pub(crate) fn check_function_for_of_statement(
    for_of_statement: ParsedForOfStatement,
    statement_index: usize,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let _barrier = ContinueTarget::enter();
    let flow_active = flow_state.tracked_local_count() > 0;
    let head = crate::flow::for_head_initializing(&for_of_statement);
    let iterable_blocked = if flow_active || !head.is_empty() {
        let mark = flow_state.begin_initializer(head);
        let blocked = check_expression_flow(
            &for_of_statement.iterable,
            for_of_statement.iterable_span,
            flow_state,
            statement_index,
            ctx,
        );
        flow_state.end_initializer(mark);
        blocked
    } else {
        FlowCheck::Clear
    };

    // A `let`/`const` head is declared by the statement, so the expression it
    // iterates already sees it — only a deferred read can get past the
    // temporal dead zone reported above, and its type is circular there.
    // A `var` head is hoisted, so the expression reads the variable the head
    // declares; unless an earlier declaration already typed it, its type
    // depends on that expression (`getTypeForVariableLikeDeclaration`) and is
    // circular there — `any` (`reportCircularityError`).
    let head_var_circular = for_of_statement.binding_kind
        == surge_ts_syntax::ParsedForBindingKind::Var
        && matches!(
            &for_of_statement.binding_name,
            surge_ts_syntax::ParsedBindingName::Identifier { name, .. } if scopes.resolve(name).is_none()
        );
    let head_var_self_read = match &for_of_statement.binding_name {
        surge_ts_syntax::ParsedBindingName::Identifier { name, span: Some(span) }
            if for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::Var
                && scopes.resolve(name).is_none_or(|symbol| symbol.ty.is_unknown()) =>
        {
            let mut reads = crate::checks::function::ValueReads::default();
            crate::checks::function::collect_value_reads(&for_of_statement.iterable, false, &mut reads);
            reads.eager.contains(name).then(|| (name.clone(), *span))
        }
        _ => None,
    };
    let head_scoped = head_var_circular
        || for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::BlockScoped;
    if head_scoped {
        scopes.push_child();
        insert_binding_name(&for_of_statement.binding_name, Type::Any, scopes);
    }
    let mut element_type = Type::Unknown;
    let mut numeric_property_names = false;
    // An existing binding at the head is checked as the expression it is
    // (`checkExpression(varExpr)` in `checkForInStatement` and
    // `checkForOfStatement`), so a name that resolves to nothing is reported.
    let head_name = match (&for_of_statement.binding_kind, &for_of_statement.binding_name) {
        (surge_ts_syntax::ParsedForBindingKind::ExistingBinding, surge_ts_syntax::ParsedBindingName::Identifier { name, span }) => {
            Some((name.clone(), *span))
        }
        _ => None,
    };
    for (name, span) in head_name.iter().chain(&for_of_statement.head_names) {
        if scopes.resolve(name).is_none()
            && !ctx
                .module_value_fallback
                .as_ref()
                .is_some_and(|fallback| fallback.get(name).is_some())
        {
            let visible_symbols = visible_symbols(scopes);
            crate::checks::expr::report_unresolved_value_name(
                name,
                *span,
                crate::checks::expr::UnresolvedNameSite::Reference,
                &visible_symbols,
                ctx,
            );
        }
    }
    // The head is written each iteration: a constant, function, class or enum
    // there is rejected (`checkIdentifier` on an assignment target), and its
    // type is then not compared.
    let rejected_head = head_name.as_ref().is_some_and(|(name, span)| {
        let Some(diagnostic) = scopes
            .resolve(name)
            .and_then(|symbol| crate::checks::assign::unwritable_binding_diagnostic(name, symbol, ctx))
        else {
            return false;
        };
        ctx.push(crate::spans::diagnostic_with_syntax_span(diagnostic, *span));
        true
    });
    if let Some((target, span)) = &for_of_statement.head_target {
        let visible_symbols = visible_symbols(scopes);
        let _ = evaluate_expression(target, *span, &visible_symbols, ctx);
        // A member head is written each iteration (`checkReferenceExpression`);
        // the head is that member itself, not a default inside a pattern.
        if let Some(span) = span
            && matches!(
                for_of_statement.binding_name,
                surge_ts_syntax::ParsedBindingName::Unsupported { span: Some(head) } if head == *span
            )
            && matches!(
                target,
                surge_ts_syntax::ParsedExpression::PropertyAccess { .. }
                    | surge_ts_syntax::ParsedExpression::ElementAccess { .. }
                    | surge_ts_syntax::ParsedExpression::IndexAccess { .. }
            )
        {
            crate::checks::expr::report_readonly_member_write(target, *span, &visible_symbols, ctx);
        }
    }
    if !iterable_blocked.is_blocked() {
        let visible_symbols = visible_symbols(scopes);
        let iterable_type = evaluate_expression(
            &for_of_statement.iterable,
            for_of_statement.iterable_span,
            &visible_symbols,
            ctx,
        );
        // `for (k in o)` binds the property key, which is always `string` — the
        // right-hand side is still evaluated so errors inside it surface. The
        // key is *indexed* as a number when the iterated object's only index
        // signature is numeric, which is a property of the access rather than
        // of the binding (tsc: `isForInVariableForNumericPropertyNames`).
        if for_of_statement.keys_only {
            if let InferredExpression::Known(iterable_type) = &iterable_type {
                check_for_in_right_operand(
                    &for_of_statement.iterable,
                    iterable_type,
                    for_of_statement.iterable_span,
                    ctx,
                );
                if for_of_statement.binding_kind
                    == surge_ts_syntax::ParsedForBindingKind::ExistingBinding
                    && !rejected_head
                    && let surge_ts_syntax::ParsedBindingName::Identifier {
                        name,
                        span: Some(span),
                    } = &for_of_statement.binding_name
                    && let Some(left_type) = visible_symbols.get(name).map(|symbol| symbol.ty.clone())
                {
                    check_for_in_left_operand(&left_type, iterable_type, *span, ctx);
                }
            }
            // Over a type variable tsc binds the generic `Extract<keyof T,
            // string>` (`getIndexTypeOrString`), which a write through it
            // defers on rather than landing on an index signature. Over other
            // generic shapes, and one surge could not model, it cannot tell.
            element_type = match &iterable_type {
                InferredExpression::Known(iterable_type)
                    if iterable_type.is_type_variable()
                        && let Some(keys) = surge_ts_types::type_variable::extract_string_keys(iterable_type) =>
                {
                    keys
                }
                InferredExpression::Known(iterable_type)
                    if has_generic_index_type(iterable_type) || iterable_type.is_unmodelled() =>
                {
                    Type::Unknown
                }
                _ => Type::String,
            };
            numeric_property_names = matches!(
                &iterable_type,
                InferredExpression::Known(iterable_type)
                    if has_numeric_property_names(iterable_type)
            );
        } else {
            crate::checks::expr::check_property_receiver(
                &for_of_statement.iterable,
                &iterable_type,
                for_of_statement.iterable_span,
                None,
                &visible_symbols,
                ctx,
            );
            // Without `strictNullChecks`, `unknown` reaches the iteration
            // protocol check instead of `checkNonNullType`.
            if !surge_ts_types::strict_null_checks()
                && matches!(
                    iterable_type,
                    InferredExpression::Known(Type::GenuineUnknown)
                )
            {
                let diagnostic = if for_of_statement.is_await {
                    Diagnostic::ts2504("unknown", ctx.file_name.clone())
                } else {
                    Diagnostic::ts2488("unknown", ctx.file_name.clone())
                };
                ctx.push(crate::spans::diagnostic_with_syntax_span(
                    diagnostic,
                    for_of_statement.iterable_span,
                ));
            }
            if for_of_statement.is_await {
                crate::checks::expr::check_async_iterable_operand(
                    &iterable_type,
                    for_of_statement.iterable_span,
                    true,
                    ctx,
                );
            } else {
                crate::checks::expr::check_iterable_operand(
                    &iterable_type,
                    for_of_statement.iterable_span,
                    false,
                    ctx,
                );
            }
            crate::checks::expr::check_iteration_next_type(
                &iterable_type,
                for_of_statement.iterable_span,
                &Type::Undefined,
                crate::checks::expr::IterationSend::ForOf,
                for_of_statement.is_await,
                ctx,
            );
            if let InferredExpression::Known(iterable_type) = iterable_type {
                element_type = if for_of_statement.is_await {
                    for_await_element_type(&iterable_type, ctx)
                } else {
                    for_of_element_type(&iterable_type)
                };
            }
            // `checkForOfStatement`: a head naming an existing binding is
            // assigned each iterated value, which must fit its declared type.
            if for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::ExistingBinding
                && !rejected_head
                && let surge_ts_syntax::ParsedBindingName::Identifier {
                    name,
                    span: Some(span),
                } = &for_of_statement.binding_name
                && let Some(target_type) = visible_symbols
                    .declared_type(name)
                    .or_else(|| visible_symbols.get(name).map(|symbol| &symbol.ty))
                && !crate::checks::function::type_contains_degradation(&element_type)
                && !crate::checks::function::type_contains_degradation(target_type)
                && !surge_ts_types::is_assignable_to(&element_type, target_type)
            {
                ctx.push(crate::spans::diagnostic_with_syntax_span(
                    Diagnostic::ts2322(element_type.name(), target_type.name(), ctx.file_name.clone()),
                    Some(*span),
                ));
            }
            // A destructuring head is `checkDestructuringAssignment` of each
            // iterated value, lowered as assignments that read it by name.
            if !for_of_statement.is_await
                && !for_of_statement.head_assignments.is_empty()
                && !crate::checks::function::type_contains_degradation(&element_type)
            {
                let mut element_symbols =
                    visible_symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
                let _ = element_symbols.insert(
                    surge_ts_syntax::FOR_OF_HEAD_ELEMENT,
                    crate::symbols::SymbolInfo {
                        ty: element_type.clone(),
                        kind: crate::symbols::SymbolKind::Const,
                        function_signature: None,
                    },
                );
                for assignment in &for_of_statement.head_assignments {
                    let shadowed_locally = scopes.declares_locally(&assignment.target_name);
                    let _ = crate::checks::assign::check_assignment_with_symbols(
                        assignment.clone(),
                        &element_symbols,
                        shadowed_locally,
                        ctx,
                    );
                }
            }
        }
    }
    if head_scoped {
        scopes.pop_child();
    }
    if let Some((name, span)) = head_var_self_read {
        element_type = Type::Any;
        if ctx.options.no_implicit_any {
            ctx.push(crate::spans::diagnostic_with_syntax_span(
                Diagnostic::ts7022(&name, ctx.file_name.clone()),
                Some(span),
            ));
        }
    }

    let assigned = loop_join_names(&for_of_statement.body);
    scopes.push_child();
    // `for (const _ in ref)` acts as a non-null assertion on `ref` for the
    // duration of the body (tsc: `getTypeAtFlowNode`, flow.go — "for (const _
    // in ref) acts as a nonnull on ref"), so indexing an optional object with
    // the key it just produced is not a possibly-undefined access.
    if for_of_statement.keys_only {
        narrow_reference_non_null_in_scope(&for_of_statement.iterable, scopes);
        // ...and on what an optional chain there reads through
        // (`optionalChainContainsReference`): `obj.main` in `obj.main?.childs`.
        for contained in super::super::narrowing::optional_chain_contained_references(
            &for_of_statement.iterable,
        ) {
            narrow_reference_non_null_in_scope(contained, scopes);
        }
    }
    let loop_assigned_name = match (
        &for_of_statement.binding_name,
        for_of_statement.binding_kind,
    ) {
        (
            surge_ts_syntax::ParsedBindingName::Identifier { name, .. },
            surge_ts_syntax::ParsedForBindingKind::Var
            | surge_ts_syntax::ParsedForBindingKind::ExistingBinding,
        ) => Some(name.clone()),
        _ => None,
    };
    // A hoisted `var` whose element type already admits `undefined` is never
    // reported (tsc's `containsUndefinedType` gate), however the loop runs.
    if for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::Var
        && let Some(name) = &loop_assigned_name
        && crate::flow::type_assumed_initialized(&element_type, ctx)
    {
        flow_state.mark_assigned(name);
    }
    // `checkVariableLikeDeclaration` for a `var` head: a `for…in` declares the
    // key type (`getIndexTypeOrString`), which an earlier declaration of the
    // `var` must be identical to (TS2403). Over `this` in an instance member
    // the key type is `Extract<keyof this, string>`, which no other is.
    if for_of_statement.keys_only
        && for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::Var
        && let surge_ts_syntax::ParsedBindingName::Identifier { name, span: Some(span) } = &for_of_statement.binding_name
        && let Some(previous) = scopes
            .resolve(name)
            .filter(|symbol| matches!(symbol.kind, crate::symbols::SymbolKind::Var) && !symbol.ty.is_degraded())
            .map(|symbol| symbol.ty.clone())
    {
        let over_this = matches!(for_of_statement.iterable, surge_ts_syntax::ParsedExpression::This { .. })
            && scopes
                .resolve("this")
                .is_some_and(|this| matches!(this.ty, Type::Reference(_) | Type::Object(_)) && !this.ty.name().starts_with("typeof "));
        let head_name = if over_this {
            Some("Extract<keyof this, string>".to_string())
        } else if matches!(element_type, Type::String) && !surge_ts_types::is_type_identical_to(&previous, &Type::String) {
            Some("string".to_string())
        } else {
            None
        };
        if let Some(head_name) = head_name {
            ctx.push(crate::spans::diagnostic_with_syntax_span(
                Diagnostic::ts2403(name, previous.name(), head_name, ctx.file_name.clone()),
                Some(*span),
            ));
        }
    }
    // A declared destructuring head reads its names off each value
    // (`checkVariableLikeDeclaration`); a `for…in` key is a `string`. An array
    // literal iterated is contextually typed by the pattern, which lends its
    // elements the names the pattern defaults.
    if for_of_statement.binding_kind != surge_ts_syntax::ParsedForBindingKind::ExistingBinding
        && !(!for_of_statement.keys_only
            && matches!(for_of_statement.iterable, surge_ts_syntax::ParsedExpression::ArrayLiteral { .. }))
        && matches!(
            for_of_statement.binding_name,
            surge_ts_syntax::ParsedBindingName::ObjectPattern(_) | surge_ts_syntax::ParsedBindingName::ArrayPattern(_)
        )
        && !element_type.is_unmodelled()
        && !crate::checks::function::type_contains_degradation(&element_type)
    {
        crate::infer::types::check_binding_pattern_reads(&for_of_statement.binding_name, &element_type, ctx);
    }
    let head_type = element_type.clone();
    insert_binding_name(&for_of_statement.binding_name, element_type, scopes);
    // A `var` head is function-scoped: it stays visible after the loop.
    if for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::Var
        && let Some(name) = &loop_assigned_name
        && let Some(symbol) = scopes.resolve(name).cloned()
    {
        scopes.insert_current(
            name.as_str(),
            crate::symbols::SymbolInfo {
                kind: crate::symbols::SymbolKind::Var,
                ..symbol
            },
        );
    }
    // The key of a `for…in` over a numerically-keyed object indexes as a
    // number, so an access through it reads the numeric index signature rather
    // than reporting an implicit `any`.
    if numeric_property_names
        && let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } =
            &for_of_statement.binding_name
        && let Some(symbol) = scopes.resolve(name).cloned()
    {
        scopes.insert_current(
            name.as_str(),
            crate::symbols::SymbolInfo {
                kind: crate::symbols::SymbolKind::ForInNumericKey,
                ..symbol
            },
        );
    }
    // The pre-pass reads the loop binding, so it runs once that is in scope;
    // head types land on the declaring frames, outside this child.
    widen_loop_assigned_bindings(&for_of_statement.body, return_type, scopes, flow_state, ctx);
    let entry_types = branch_assignment_types(&assigned, scopes);
    // The head is assigned at the top of every iteration
    // (`bindForInOrForOfStatement`), after the back edge joins: whatever the
    // body writes to it, the body starts from the element.
    if let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } = &for_of_statement.binding_name
        && let Some(symbol) = scopes.resolve(name).cloned()
        && symbol.ty != head_type
    {
        scopes.insert_current(
            name.as_str(),
            crate::symbols::SymbolInfo {
                ty: head_type,
                ..symbol
            },
        );
    }
    let pending_mutations = prime_loop_mutations(&for_of_statement.body, scopes, ctx);
    if flow_active {
        flow_state.begin_branch_capture();
        // Each iteration assigns the head before the body runs; the loop may
        // run zero times, so this stays inside the body's branch.
        if let Some(name) = &loop_assigned_name {
            flow_state.mark_assigned(name);
        }
        for (name, _) in &for_of_statement.head_names {
            flow_state.mark_assigned(name);
        }
        check_function_body(for_of_statement.body, return_type, scopes, flow_state, ctx);
        let _ = flow_state.finish_branch_capture();
    } else {
        check_function_body(for_of_statement.body, return_type, scopes, flow_state, ctx);
    }
    release_loop_mutations(pending_mutations, scopes, ctx);
    let body_types = branch_assignment_types(&assigned, scopes);
    // A numeric `for…in` key indexes as a number only inside the loop; the
    // `var` it declares outlives it as the `string` it is.
    if numeric_property_names
        && for_of_statement.binding_kind == surge_ts_syntax::ParsedForBindingKind::Var
        && let Some(name) = &loop_assigned_name
        && let Some(symbol) = scopes.resolve(name).cloned()
        && matches!(symbol.kind, crate::symbols::SymbolKind::ForInNumericKey)
    {
        scopes.insert_current(
            name.as_str(),
            crate::symbols::SymbolInfo {
                kind: crate::symbols::SymbolKind::Var,
                ..symbol
            },
        );
    }
    scopes.pop_child();
    join_branch_pair(&entry_types, &body_types, scopes);
}

/// Whether `keyof ty` stays generic (`shouldDeferIndexType`): `ty` is a type
/// variable, or a union or intersection with one among its constituents
/// (`(T & object) | (T & null)` under a `typeof` guard).
fn has_generic_index_type(ty: &Type) -> bool {
    match ty {
        _ if ty.is_type_variable() => true,
        Type::Union(union) => union.types().iter().any(has_generic_index_type),
        Type::Object(object) => object
            .intersection_operands
            .as_deref()
            .is_some_and(|operands| operands.iter().any(has_generic_index_type)),
        _ => false,
    }
}

/// tsc's `checkForInStatement` right-hand rule (TS2407): with `null` and
/// `undefined` removed under strictNullChecks, the operand must be `any`, an
/// object type or a type parameter. The `null` keyword leaves nothing, so it
/// reads as `never`.
fn check_for_in_right_operand(
    operand: &surge_ts_syntax::ParsedExpression,
    operand_type: &Type,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    fn is_object_like(ty: &Type) -> bool {
        match ty.peeled() {
            Type::Any
            | Type::Object(_)
            | Type::Function(_)
            | Type::Array(_)
            | Type::Tuple(_)
            | Type::OpenTuple(_) => true,
            Type::Union(union) => union.types().iter().all(is_object_like),
            _ => false,
        }
    }
    // Without strictNullChecks `getNonNullableTypeIfNeeded` keeps `null` and
    // `undefined`, both assignable to `object`.
    if !surge_ts_types::strict_null_checks()
        && (matches!(operand, surge_ts_syntax::ParsedExpression::NullLiteral)
            || matches!(operand_type.peeled(), Type::Null | Type::Undefined))
    {
        return;
    }
    let right_type = if matches!(operand, surge_ts_syntax::ParsedExpression::NullLiteral) {
        Type::Never
    } else {
        let peeled = operand_type.peeled();
        let degraded =
            |ty: &Type| matches!(ty, Type::Unknown | Type::ErrorType | Type::TypeParameter(_));
        if degraded(&peeled)
            || matches!(&peeled, Type::Union(union) if union.types().iter().any(degraded))
        {
            return;
        }
        match peeled {
            Type::Undefined => Type::Never,
            _ => surge_ts_types::remove_undefined(operand_type),
        }
    };
    if right_type != Type::Never && is_object_like(&right_type) {
        return;
    }
    let span = span
        .and_then(|span| ctx.parenthesized_outer_span(span))
        .or(span);
    ctx.push(crate::spans::diagnostic_with_syntax_span(
        Diagnostic::ts2407(right_type.name(), ctx.file_name.clone()),
        span,
    ));
}

/// tsc's `checkForInStatement` left-hand rule (TS2405): `for (x in o)` over an
/// existing binding needs `getIndexTypeOrString(o)` assignable to `x`.
fn check_for_in_left_operand(
    left_type: &Type,
    right_type: &Type,
    span: surge_ts_syntax::TextSpan,
    ctx: &mut CheckerContext,
) {
    if left_type.is_unknown() {
        return;
    }
    let Some(index_type) = index_type_or_string(right_type) else {
        return;
    };
    if surge_ts_types::is_assignable_to(&index_type, left_type) {
        return;
    }
    ctx.push(crate::spans::diagnostic_with_syntax_span(
        Diagnostic::ts2405(ctx.file_name.clone()),
        Some(span),
    ));
}

/// tsc's `getIndexTypeOrString`: `Extract<keyof T, string>`, or `string` when
/// that is `never`. A shape whose keys surge does not enumerate — an array, a
/// callable, a type variable, a primitive, a string-indexed object — reads as
/// `string`, which relates like the real key set to every target but a
/// literal one. `None` for an operand surge could not resolve.
fn index_type_or_string(right_type: &Type) -> Option<Type> {
    // A numeric-looking key is a number literal in `keyof` when it was written
    // as a number and a string literal when quoted; surge keeps no record of
    // which, so such a shape is left unanswered.
    fn string_keys(object: &surge_ts_types::ObjectType) -> Option<Vec<String>> {
        let keys: Vec<String> = object
            .properties
            .keys()
            .filter(|key| !key.starts_with('['))
            .map(|key| key.to_string())
            .collect();
        (!keys.iter().any(|key| key.parse::<f64>().is_ok())).then_some(keys)
    }
    let right_type = surge_ts_types::remove_nullish(right_type).peeled();
    let keys = match &right_type {
        Type::Unknown | Type::ErrorType => return None,
        Type::Object(object) if object.string_index_type.is_none() => string_keys(object)?,
        // `keyof (A | B)` is the keys common to every member.
        Type::Union(union) => {
            let mut common: Option<Vec<String>> = None;
            for member in union.types() {
                let member_keys = match member.peeled() {
                    Type::Unknown | Type::ErrorType => return None,
                    Type::Object(object) if object.string_index_type.is_some() => continue,
                    Type::Object(object) => string_keys(&object)?,
                    _ => Vec::new(),
                };
                common = Some(match common {
                    None => member_keys,
                    Some(keys) => keys
                        .into_iter()
                        .filter(|key| member_keys.contains(key))
                        .collect(),
                });
            }
            common.unwrap_or_default()
        }
        _ => Vec::new(),
    };
    Some(if keys.is_empty() {
        Type::String
    } else {
        surge_ts_types::union_type(keys.into_iter().map(Type::StringLiteral).collect())
    })
}

/// tsc's `hasNumericPropertyNames`: the type's only index signature is the
/// numeric one, which is what makes a `for…in` key over it a `number`.
fn has_numeric_property_names(ty: &Type) -> bool {
    match ty.peeled() {
        Type::Object(object) => object.has_numeric_property_names(),
        // An array or tuple is indexed by number and nothing else, which is the
        // whole point of the rule: `for (const k in xs) xs[k]` is the idiom it
        // exists for.
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        _ => false,
    }
}

pub(crate) fn for_of_element_type(iterable_type: &Type) -> Type {
    match iterable_type {
        Type::Any => Type::Any,
        Type::Array(element) => with_type_copy_reason(TypeCopyReason::FunctionBodySetup, || {
            element.as_ref().clone()
        }),
        Type::Tuple(elements) => {
            if elements.is_empty() {
                Type::Unknown
            } else {
                with_type_copy_reason(TypeCopyReason::FunctionBodySetup, || {
                    union_type(elements.clone())
                })
            }
        }
        Type::String | Type::StringLiteral(_) => Type::String,
        Type::Union(union) => {
            let element_types = union
                .types()
                .iter()
                .filter(|ty| !matches!(ty, Type::Undefined | Type::Null))
                .map(for_of_element_type)
                .collect::<Vec<_>>();

            if element_types.is_empty() {
                Type::Unknown
            } else {
                union_type(element_types)
            }
        }
        // A nominal collection/iterator reference (`Set<T>`, `Map<K, V>`,
        // `MapIterator<V>`, …) yields its element type from the resolved type
        // arguments, without forcing the whole lib iterator graph to expand.
        Type::Reference(reference) => {
            if let Some(element) = iterable_reference_element_type(reference) {
                element
            } else if let Some(types) = structural_iteration_types(iterable_type) {
                types.yield_type
            } else {
                // A non-collection reference may still be a structural iterable
                // (an array alias, a tuple alias). Peel once and re-derive; the
                // peeled shape is never another reference for these, so this does
                // not loop.
                match reference.resolve() {
                    Type::Reference(_) => Type::Unknown,
                    peeled => for_of_element_type(&peeled),
                }
            }
        }
        Type::Object(_) => structural_iteration_types(iterable_type)
            .map(|types| types.yield_type)
            .unwrap_or(Type::Unknown),
        _ => Type::Unknown,
    }
}

/// `checkForOfStatement` for `for await`: the async iteration protocol first,
/// then the sync one with its values awaited.
fn for_await_element_type(iterable_type: &Type, ctx: &mut CheckerContext) -> Type {
    let usage = crate::checks::expr::IterationUse::for_await(ctx.options.strict_builtin_iterator_return);
    let element = match crate::checks::expr::iteration_types_of_iterable(iterable_type, usage) {
        Some(types) if !types.yield_type.is_unmodelled() => types.yield_type,
        _ => for_of_element_type(iterable_type),
    };
    crate::checks::call::awaited_type_with_alias(&element, ctx)
}

/// The iteration types of an operand the nominal fast paths above do not
/// name. Only the yield type is read here, which the `BuiltinIteratorReturn`
/// setting does not affect.
fn structural_iteration_types(iterable_type: &Type) -> Option<crate::checks::expr::IterationTypes> {
    crate::checks::expr::iteration_types_of_iterable(
        iterable_type,
        crate::checks::expr::IterationUse::sync(false),
    )
    .filter(|types| !types.yield_type.is_unmodelled())
}

/// The element type a `for…of` binds when iterating a known lib collection or
/// iterator reference, derived from its resolved type arguments. `Map`-like
/// references yield the `[K, V]` entry tuple; `Set`-like and the iterator
/// wrappers yield their single element argument. Returns `None` for any other
/// reference so the caller can fall back to structural peeling.
pub(crate) fn iterable_reference_element_type(
    reference: &surge_ts_types::TypeReference,
) -> Option<Type> {
    let name = reference.id.rsplit('\u{0}').next().unwrap_or(&reference.id);
    let arg = |index: usize| reference.arguments.get(index).cloned();
    match name {
        "Map" | "ReadonlyMap" | "WeakMap" => match (arg(0), arg(1)) {
            (Some(key), Some(value)) => Some(Type::Tuple(vec![key, value])),
            _ => Some(Type::Unknown),
        },
        "Set" | "ReadonlySet" | "WeakSet" => Some(arg(0).unwrap_or(Type::Unknown)),
        "IterableIterator"
        | "Iterator"
        | "IteratorObject"
        | "ArrayIterator"
        | "MapIterator"
        | "SetIterator"
        | "Generator"
        | "AsyncGenerator"
        | "IterableIteratorObject" => Some(arg(0).unwrap_or(Type::Unknown)),
        _ => None,
    }
}

/// The clauses TS7029 may report: every non-empty clause but the last (empty
/// clauses, stacked `case` labels, are allowed to fall through), with the call
/// each ends with.
fn fallthrough_candidates(
    switch_statement: &ParsedSwitchStatement,
) -> Vec<(Option<surge_ts_syntax::TextSpan>, Vec<ParsedFunctionBodyStatement>, Option<(usize, usize)>)> {
    let case_count = switch_statement.cases.len();
    switch_statement
        .cases
        .iter()
        .enumerate()
        .filter(|(index, case)| index + 1 != case_count && !case.consequent.is_empty())
        .map(|(_, case)| {
            (
                case.span,
                case.consequent.clone(),
                crate::checks::expr::tail_call_key(&case.consequent),
            )
        })
        .collect()
}

/// TS7029 under `noFallthroughCasesInSwitch`: a clause whose end is reachable
/// (`isReachableFlowNode` of its fallthrough flow) falls through into the next
/// one. Decided once the clauses are checked, so a nested `switch` known to be
/// exhaustive and a call that returns `never` end the flow as they do for tsc.
fn emit_switch_fallthrough_diagnostics(
    candidates: Vec<(Option<surge_ts_syntax::TextSpan>, Vec<ParsedFunctionBodyStatement>, Option<(usize, usize)>)>,
    ctx: &mut CheckerContext,
) {
    for (span, consequent, tail_call) in candidates {
        if tail_call.is_some_and(|key| ctx.never_returning_calls.contains(&key)) {
            continue;
        }
        let flow = crate::flow::with_non_exhaustive_switches(
            &ctx.non_exhaustive_switches,
            &ctx.exhaustive_switches,
            || analyze_function_body_flow(&consequent),
        );
        if !flow.guarantees_exit {
            let diagnostic = Diagnostic::ts7029(ctx.file_name.clone());
            let diagnostic = match span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            };
            ctx.push(diagnostic);
        }
    }
}

/// A `case` test is an expression of its own — whatever is written there is
/// checked — and its type has to be comparable to the discriminant's, the same
/// relation `===` uses (TS2678). A `default` clause has no test.
fn check_switch_case_tests(
    switch_statement: &ParsedSwitchStatement,
    discriminant: &InferredExpression,
    symbols: &crate::symbols::SymbolTable,
    ctx: &mut CheckerContext,
) {
    // The comparison uses the discriminant's *declared* type when it has one.
    // A `let` the source narrows through a closure keeps its declared union in
    // tsc, and surge's flow state can be narrower than the source really
    // guarantees — comparing against the narrowed type there reports a case the
    // program does reach.
    let declared_discriminant = match &switch_statement.discriminant {
        ParsedExpression::Identifier { name, .. } => symbols.declared_type(name),
        _ => None,
    };

    for switch_case in &switch_statement.cases {
        let Some(test) = switch_case.test.as_ref() else {
            continue;
        };
        let inferred_case = evaluate_expression(test, switch_case.test_span, symbols, ctx);
        let (Some(case_type), Some(discriminant_type)) = (
            crate::checks::ops::inferred_type(&inferred_case),
            declared_discriminant.or_else(|| crate::checks::ops::inferred_type(discriminant)),
        ) else {
            continue;
        };
        // Same exemptions the `===` check makes: a degraded or `any` operand
        // says nothing, and a nullish test is comparable to anything.
        if case_type.is_unknown()
            || discriminant_type.is_unknown()
            || matches!(case_type, Type::Any | Type::Undefined | Type::Null)
            || matches!(discriminant_type, Type::Any | Type::Undefined | Type::Null)
        {
            continue;
        }
        // A fresh object literal test with a member the discriminant's object
        // type lacks is not comparable either way; the reversed
        // `checkTypeComparableTo` reports it as an excess property.
        if let ParsedExpression::ObjectLiteral { properties, span } = test
            && !crate::checks::function::type_contains_degradation(discriminant_type)
            && let Type::Object(object) = discriminant_type.peeled()
            && crate::checks::expected::report_excess_property(
                properties,
                &object,
                span.or(switch_case.test_span),
                ctx,
            )
        {
            continue;
        }
        if crate::checks::ops::types_overlap_for_equality(case_type, discriminant_type) {
            continue;
        }
        // A relation error, not TS2367's pairing: the discriminant is named as
        // declared and the case widens only against a literal-free target.
        let case_name = crate::checks::expr::source_display_name(case_type, discriminant_type);
        let discriminant_name = discriminant_type.name();
        let diagnostic = Diagnostic::ts2678(&case_name, &discriminant_name, ctx.file_name.clone());
        let diagnostic = match switch_case.test_span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        };
        ctx.push(diagnostic);
    }
}

/// Which case tests the no-match condition of a `switch` over `discriminant`
/// keeps: all of them, but an `undefined` test (`"undefined"` under `typeof`)
/// when the discriminant reads through an optional chain.
fn no_match_keeps_test(discriminant: &ParsedExpression) -> impl Fn(&ParsedExpression) -> bool {
    let (chain, under_typeof) = match discriminant {
        ParsedExpression::Unary {
            operator: surge_ts_syntax::ParsedUnaryOperator::Typeof,
            operand,
            ..
        } => (operand.as_ref(), true),
        other => (other, false),
    };
    let optional_chain = !crate::checks::function::narrowing::optional_chain_contained_references(chain).is_empty();
    move |test: &ParsedExpression| {
        !optional_chain
            || !match test {
                ParsedExpression::StringLiteral(value) => under_typeof && value == "undefined",
                ParsedExpression::UndefinedLiteral => !under_typeof,
                ParsedExpression::Identifier { name, .. } => !under_typeof && name == "undefined",
                _ => false,
            }
    }
}

pub(crate) fn check_function_switch_statement(
    mut switch_statement: ParsedSwitchStatement,
    statement_index: usize,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    // A template the constant evaluator spells out is a string literal to tsc
    // (`checkTemplateExpression`), as a case test too: `case \`number\`:` under
    // `switch (typeof x)`, `case \`${Kind.a}\`:` under a discriminant.
    let case_symbols = visible_symbols(scopes);
    for case in &mut switch_statement.cases {
        if let Some(ParsedExpression::TemplateLiteral {
            expressions,
            quasis,
            ..
        }) = &case.test
            && let Type::StringLiteral(text) = crate::infer::expression::template_literal_type(
                expressions,
                quasis,
                &case_symbols,
                ctx,
            )
        {
            case.test = Some(ParsedExpression::StringLiteral(text));
        }
    }
    let fallthrough_candidates = ctx
        .options
        .no_fallthrough_cases_in_switch
        .then(|| fallthrough_candidates(&switch_statement));

    let flow_active = flow_state.tracked_local_count() > 0;
    let condition_blocked = if flow_active {
        check_expression_flow(
            &switch_statement.discriminant,
            switch_statement.discriminant_span,
            flow_state,
            statement_index,
            ctx,
        )
    } else {
        FlowCheck::Clear
    };

    if !condition_blocked.is_blocked() {
        let visible_symbols = visible_symbols(scopes);
        let discriminant = evaluate_expression(
            &switch_statement.discriminant,
            switch_statement.discriminant_span,
            &visible_symbols,
            ctx,
        );
        check_switch_case_tests(&switch_statement, &discriminant, &visible_symbols, ctx);
    }

    // `switch (x.kind) case "a":` narrows the case body exactly like
    // `if (x.kind === "a")`. A fall-through group (`case "a": case "b": body`)
    // narrows to the OR of the group's tests, matching tsc. Synthesize the
    // equality/OR condition and reuse the if-branch narrowing.
    let discriminant = switch_statement.discriminant.clone();
    let discriminant_span = switch_statement.discriminant_span;
    // `narrowTypeBySwitchOnTrue`: under `switch (true)` a case narrows by its
    // own expression, and the cases before it by theirs failing.
    let switch_on_true = matches!(discriminant, ParsedExpression::BooleanLiteral(true));
    let equality_condition = |test: &ParsedExpression| {
        if switch_on_true {
            return test.clone();
        }
        ParsedExpression::Binary {
            left: Box::new(discriminant.clone()),
            left_span: discriminant_span,
            operator: surge_ts_syntax::ParsedBinaryOperator::StrictEquals,
            operator_span: None,
            right: Box::new(test.clone()),
            right_span: None,
        }
    };
    let any_of = |tests: &mut dyn Iterator<Item = &ParsedExpression>| {
        tests
            .map(equality_condition)
            .reduce(|left, right| ParsedExpression::Logical {
                left: Box::new(left),
                left_span: None,
                operator: surge_ts_syntax::ParsedLogicalOperator::Or,
                operator_span: None,
                right: Box::new(right),
                right_span: None,
                truthiness_tests: Vec::new(),
            })
    };
    // `narrowTypeBySwitchOptionalChainContainment` never narrows the chain's
    // references in a `default` clause or past the `switch` (its clause type
    // is `never`), so a test there that the chain is `undefined` must not
    // make the synthesized no-match condition imply it is defined.
    let keeps_in_no_match = no_match_keeps_test(&discriminant);
    // Matching none of the cases is what the `default` clause, and the code
    // after a `switch` without one, sees (`narrowTypeBySwitchOnDiscriminant`).
    let no_case_matches = any_of(
        &mut switch_statement
            .cases
            .iter()
            .filter_map(|case| case.test.as_ref())
            .filter(|test| keeps_in_no_match(test)),
    );
    // Per case: the tests of the maximal run of empty-consequent cases falling
    // into it, plus its own test, and the branch of that condition the body
    // runs in. A group containing `default` runs wherever no case *outside* the
    // group matched.
    let case_group_conditions: Vec<Option<(ParsedExpression, bool)>> = {
        let mut group: Vec<Option<&ParsedExpression>> = Vec::new();
        switch_statement
            .cases
            .iter()
            .enumerate()
            .map(|(case_index, switch_case)| {
                group.push(switch_case.test.as_ref());
                let condition = if group.iter().any(|test| test.is_none()) {
                    any_of(&mut switch_statement.cases.iter().filter_map(|case| {
                        case.test.as_ref().filter(|test| {
                            keeps_in_no_match(test)
                                && !group
                                    .iter()
                                    .flatten()
                                    .any(|member| std::ptr::eq(*member, *test))
                        })
                    }))
                    .map(|condition| (condition, false))
                } else {
                    any_of(&mut group.iter().filter_map(|test| *test)).map(|condition| {
                        let preceding = switch_on_true
                            .then(|| {
                                any_of(&mut switch_statement.cases.iter().take(case_index + 1 - group.len()).filter_map(
                                    |case| case.test.as_ref(),
                                ))
                            })
                            .flatten();
                        match preceding {
                            Some(preceding) => (
                                ParsedExpression::Logical {
                                    left: Box::new(ParsedExpression::Unary {
                                        operator: surge_ts_syntax::ParsedUnaryOperator::Not,
                                        operator_span: None,
                                        operand: Box::new(preceding),
                                        operand_span: None,
                                    }),
                                    left_span: None,
                                    operator: surge_ts_syntax::ParsedLogicalOperator::And,
                                    operator_span: None,
                                    right: Box::new(condition),
                                    right_span: None,
                                    truthiness_tests: Vec::new(),
                                },
                                true,
                            ),
                            None => (condition, true),
                        }
                    })
                };
                if !switch_case.consequent.is_empty() {
                    group.clear();
                }
                condition
            })
            .collect()
    };
    let has_default = switch_statement
        .cases
        .iter()
        .any(|case| case.test.is_none());
    let case_literals = (!has_default)
        .then(|| switch_case_literals(&switch_statement.cases, scopes, ctx))
        .flatten();
    let every_case_exits = switch_statement
        .cases
        .last()
        .is_some_and(|case| !case.consequent.is_empty())
        && switch_statement
            .cases
            .iter()
            .filter(|case| !case.consequent.is_empty())
            .all(|case| {
                // `break` leaves the case for the code after the `switch`.
                let flow = analyze_function_body_flow(&case.consequent);
                (flow.guarantees_value_return || flow.guarantees_exit)
                    && !crate::flow::body_breaks_enclosing_loop(&case.consequent)
            });

    // Every case that reaches the code after the `switch` (by `break` or by
    // running off its end) is an edge into it, and so is matching no case when
    // there is no `default`; the bindings the cases assign are joined over them.
    let mut assigned = Vec::new();
    for switch_case in &switch_statement.cases {
        nested_assigned_names(&switch_case.consequent, &mut assigned);
    }
    let case_reaches_end: Vec<bool> = switch_statement
        .cases
        .iter()
        .map(|switch_case| {
            let flow = analyze_function_body_flow(&switch_case.consequent);
            !flow.guarantees_value_return
                && (!flow.guarantees_exit
                    || crate::flow::body_breaks_enclosing_loop(&switch_case.consequent))
        })
        .collect();
    let mut edges = Vec::new();
    // The implicit `default` edge carries nothing past an exhaustive `switch`
    // (`getTypeAtFlowBranchLabel`'s `bypassFlow`), and otherwise what matching
    // no case leaves of the bindings.
    if !has_default
        && !assigned.is_empty()
        && !case_literals.as_deref().is_some_and(|literals| {
            switch_covers_discriminant(&switch_statement.discriminant, literals, scopes, ctx)
        })
    {
        scopes.push_child();
        if let Some(condition) = no_case_matches.as_ref() {
            narrow_switch_case(condition, false, scopes, flow_state, ctx);
        }
        edges.push(branch_assignment_types(&assigned, scopes));
        scopes.pop_child();
    }
    // A case is entered from the `switch` itself, and from the case before it
    // only when that one runs off its end.
    let switch_entry = branch_assignment_types(&assigned, scopes);
    let falls_into_next: Vec<bool> = switch_statement
        .cases
        .iter()
        .map(|switch_case| {
            let flow = analyze_function_body_flow(&switch_case.consequent);
            !flow.guarantees_value_return && !flow.guarantees_exit
        })
        .collect();

    // The case block is one block scope (`getContainerFlags`), so a clause
    // resolves what another clause declares.
    let mut saved_fallback = None;
    for switch_case in &switch_statement.cases {
        if let Some(previous) =
            super::super::install_deferred_block_value_fallback(&switch_case.consequent, ctx)
        {
            saved_fallback.get_or_insert(previous);
        }
    }

    if flow_active {
        let mut branch_deltas = Vec::new();

        for (case_index, switch_case) in switch_statement.cases.into_iter().enumerate() {
            let case_guarantees_value_return =
                analyze_function_body_flow(&switch_case.consequent).guarantees_value_return;
            if let Some(test) = switch_case.test.as_ref() {
                let _ = check_expression_flow(
                    test,
                    switch_case.test_span,
                    flow_state,
                    statement_index,
                    ctx,
                );
            }

            if case_index > 0 && !falls_into_next[case_index - 1] {
                restore_branch_entry(&switch_entry, scopes);
            }
            scopes.push_child();
            if let Some((condition, branch_is_true)) = case_group_conditions[case_index].as_ref() {
                narrow_switch_case(condition, *branch_is_true, scopes, flow_state, ctx);
            }
            flow_state.begin_branch_capture();
            check_function_body(
                switch_case.consequent,
                with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
                scopes,
                flow_state,
                ctx,
            );
            let mut case_delta = flow_state.finish_branch_capture();
            case_delta.continues = !case_guarantees_value_return;
            if !assigned.is_empty() && case_reaches_end[case_index] {
                edges.push(branch_assignment_types(&assigned, scopes));
            }
            scopes.pop_child();
            branch_deltas.push(case_delta);
        }

        merge_branch_deltas(flow_state, &branch_deltas, false);
    } else {
        for (case_index, switch_case) in switch_statement.cases.into_iter().enumerate() {
            if case_index > 0 && !falls_into_next[case_index - 1] {
                restore_branch_entry(&switch_entry, scopes);
            }
            scopes.push_child();
            if let Some((condition, branch_is_true)) = case_group_conditions[case_index].as_ref() {
                narrow_switch_case(condition, *branch_is_true, scopes, flow_state, ctx);
            }
            check_function_body(
                switch_case.consequent,
                with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
                scopes,
                flow_state,
                ctx,
            );
            if !assigned.is_empty() && case_reaches_end[case_index] {
                edges.push(branch_assignment_types(&assigned, scopes));
            }
            scopes.pop_child();
        }
    }
    if let Some(saved) = saved_fallback {
        ctx.module_value_fallback = saved;
    }
    restore_branch_entry(&switch_entry, scopes);
    join_branch_edges(&edges, scopes);

    // Only the implicit `default` path continues past a `switch` whose every
    // case leaves it.
    if !has_default
        && every_case_exits
        && let Some(condition) = no_case_matches.as_ref()
    {
        record_non_exhaustive_switch(
            &switch_statement.discriminant,
            switch_statement.span,
            case_literals.as_deref(),
            scopes,
            ctx,
        );
        narrow_switch_case(condition, false, scopes, flow_state, ctx);
    }
    if let Some(candidates) = fallthrough_candidates {
        emit_switch_fallthrough_diagnostics(candidates, ctx);
    }
}

/// Narrows by a `switch`'s synthesized case condition. A discriminant alias
/// (`const { kind } = obj; switch (kind)`) also narrows the object it was read
/// from, as tsc's `getCandidateDiscriminantPropertyAccess` reads it as
/// `obj.kind`.
fn narrow_switch_case(
    condition: &ParsedExpression,
    branch_is_true: bool,
    scopes: &mut ScopeStack,
    flow_state: &FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    narrow_discriminant_in_scope(condition, scopes, branch_is_true, ctx);
    if let Some(rewritten) = rewrite_discriminant_aliases(condition, flow_state) {
        narrow_discriminant_in_scope(&rewritten, scopes, branch_is_true, ctx);
    }
    narrow_tuple_destructure_siblings(condition, scopes, branch_is_true);
}

/// tsc's `computeExhaustiveSwitchStatement`: only a discriminant of a literal
/// type (`isLiteralType` — `boolean`, a unit type, or a union of them) can be
/// covered, and it is when the cases name each of its members; any other type
/// has values no finite set of cases names. A case test that is not a literal,
/// or a discriminant surge could not type, records nothing.
fn record_non_exhaustive_switch(
    discriminant: &ParsedExpression,
    span: Option<surge_ts_syntax::TextSpan>,
    case_literals: Option<&[Type]>,
    scopes: &ScopeStack,
    ctx: &mut CheckerContext,
) {
    let (Some(span), Some(case_literals)) = (span, case_literals) else {
        return;
    };
    // `switch (typeof x)` is exhaustive when its cases name every tag `x` can
    // have, not every tag `typeof` can produce.
    if let ParsedExpression::Unary {
        operator: surge_ts_syntax::ParsedUnaryOperator::Typeof,
        operand,
        ..
    } = discriminant
    {
        let InferredExpression::Known(operand_type) =
            crate::infer::infer_expression(operand, &visible_symbols(scopes), ctx)
        else {
            return;
        };
        // A top type is covered only by every tag `typeof` can produce.
        let tags = if matches!(
            operand_type.peeled(),
            Type::Any | Type::ErrorType | Type::GenuineUnknown
        ) {
            vec![
                "string",
                "number",
                "bigint",
                "boolean",
                "symbol",
                "undefined",
                "object",
                "function",
            ]
        } else {
            let Some(tags) = crate::checks::function::narrowing::typeof_tags_of(&operand_type)
            else {
                return;
            };
            tags
        };
        if tags
            .iter()
            .any(|tag| !case_literals.contains(&Type::StringLiteral((*tag).to_string())))
        {
            ctx.non_exhaustive_switches.push((span.start, span.end));
        } else {
            ctx.proven_exhaustive_switches.push((span.start, span.end));
        }
        return;
    }
    let InferredExpression::Known(discriminant_type) =
        crate::infer::infer_expression(discriminant, &visible_symbols(scopes), ctx)
    else {
        return;
    };
    let members: Vec<Type> = match discriminant_type.peeled() {
        // The degradation sentinel stands for a type tsc usually has, and so
        // does an `any` read under a receiver surge could not type (a callback
        // parameter walked without its contextual type); the end point is left
        // unreported rather than guessed reachable.
        Type::Unknown => {
            ctx.exhaustive_switches.push((span.start, span.end));
            return;
        }
        Type::Any if ctx.degraded_expected_type_depth > 0 => {
            ctx.exhaustive_switches.push((span.start, span.end));
            return;
        }
        Type::TypeParameter(_) | Type::Reference(_) => return,
        Type::Boolean => vec![Type::BooleanLiteral(true), Type::BooleanLiteral(false)],
        unit @ (Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)
        | Type::Null
        | Type::Undefined) => vec![unit.clone()],
        Type::Union(union) => {
            let mut members = Vec::new();
            for member in union.types() {
                match member {
                    Type::StringLiteral(_)
                    | Type::NumberLiteral(_)
                    | Type::BooleanLiteral(_)
                    | Type::Null
                    | Type::Undefined => members.push(member.clone()),
                    Type::Boolean => {
                        members.push(Type::BooleanLiteral(true));
                        members.push(Type::BooleanLiteral(false));
                    }
                    Type::Unknown | Type::TypeParameter(_) | Type::Reference(_) => return,
                    _ => {
                        ctx.non_exhaustive_switches.push((span.start, span.end));
                        return;
                    }
                }
            }
            members
        }
        _ => {
            ctx.non_exhaustive_switches.push((span.start, span.end));
            return;
        }
    };
    if members.iter().any(|member| !case_literals.contains(member)) {
        ctx.non_exhaustive_switches.push((span.start, span.end));
    } else {
        ctx.exhaustive_switches.push((span.start, span.end));
        ctx.proven_exhaustive_switches.push((span.start, span.end));
    }
}

/// `computeExhaustiveSwitchStatement` for a discriminant of a literal type:
/// every member of it is named by a case. Anything surge cannot type as such
/// is left not exhaustive.
fn switch_covers_discriminant(
    discriminant: &ParsedExpression,
    case_literals: &[Type],
    scopes: &ScopeStack,
    ctx: &mut CheckerContext,
) -> bool {
    if matches!(
        discriminant,
        ParsedExpression::Unary { operator: surge_ts_syntax::ParsedUnaryOperator::Typeof, .. }
    ) {
        return false;
    }
    let diagnostics_before = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(discriminant, &visible_symbols(scopes), ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    let InferredExpression::Known(discriminant_type) = inferred else {
        return false;
    };
    let peeled = discriminant_type.peeled();
    let members: &[Type] = match &peeled {
        Type::Union(union) => union.types(),
        other => std::slice::from_ref(other),
    };
    let mut units = Vec::with_capacity(members.len());
    for member in members {
        match member {
            Type::Boolean => {
                units.push(Type::BooleanLiteral(true));
                units.push(Type::BooleanLiteral(false));
            }
            Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
            | Type::Null
            | Type::Undefined => units.push(member.clone()),
            _ => return false,
        }
    }
    !units.is_empty() && units.iter().all(|unit| case_literals.contains(unit))
}

/// The literal each case tests — written, or the unit type of a `const` it
/// names (`case STATUS_DONE:`) — or `None` when any case tests something else.
fn switch_case_literals(
    cases: &[surge_ts_syntax::ParsedSwitchCase],
    scopes: &ScopeStack,
    ctx: &mut CheckerContext,
) -> Option<Vec<Type>> {
    cases
        .iter()
        .filter_map(|case| case.test.as_ref())
        .map(|test| match test {
            ParsedExpression::StringLiteral(value) => Some(Type::StringLiteral(value.clone())),
            ParsedExpression::NumberLiteral(value) => {
                Some(Type::NumberLiteral(surge_ts_types::NumberLiteralType {
                    value: value.clone(),
                }))
            }
            ParsedExpression::BooleanLiteral(value) => Some(Type::BooleanLiteral(*value)),
            ParsedExpression::NullLiteral => Some(Type::Null),
            ParsedExpression::UndefinedLiteral => Some(Type::Undefined),
            ParsedExpression::Identifier { .. } | ParsedExpression::PropertyAccess { .. } => {
                let diagnostics_before = ctx.diagnostics().len();
                let inferred = crate::infer::infer_expression(test, &visible_symbols(scopes), ctx);
                ctx.truncate_diagnostics(diagnostics_before);
                match inferred {
                    InferredExpression::Known(
                        literal @ (Type::StringLiteral(_)
                        | Type::NumberLiteral(_)
                        | Type::BooleanLiteral(_)
                        | Type::Undefined),
                    ) => Some(literal),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// Undoes the widening the handler was checked under, except for what the
/// handler itself assigned: that stands at the handler's end.
fn restore_after_catch(
    before_catch: &[(String, Type)],
    catch_body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    scopes: &mut ScopeStack,
) {
    let catch_assigned = deep_assigned_names(&[catch_body]);
    let catch_end = branch_assignment_types(&catch_assigned, scopes);
    adopt_branch_assignments(before_catch, scopes);
    adopt_branch_assignments(&catch_end, scopes);
}

/// A `finally` block runs however the `try` and `catch` ended — part-way
/// included — so a binding either assigns is read at its declared type there.
/// The types the join left for the code after the statement come back once it
/// is checked.
fn check_finalizer(
    finalizer: Vec<surge_ts_syntax::ParsedFunctionBodyStatement>,
    assigning_bodies: &[&[surge_ts_syntax::ParsedFunctionBodyStatement]],
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    if finalizer.is_empty() {
        return;
    }
    let assigned = deep_assigned_names(assigning_bodies);
    let after_try = branch_assignment_types(&assigned, scopes);
    widen_assigned_bindings(assigning_bodies, scopes);
    scopes.push_child();
    check_function_body(finalizer, return_type, scopes, flow_state, ctx);
    scopes.pop_child();
    adopt_branch_assignments(&after_try, scopes);
}

pub(crate) fn check_function_try_statement(
    try_statement: ParsedTryStatement,
    _statement_index: usize,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let flow_active = flow_state.tracked_local_count() > 0;

    if flow_active {
        let mut branch_deltas = Vec::new();
        let try_flow = analyze_function_body_flow(&try_statement.block);
        let try_guarantees_value_return = try_flow.guarantees_value_return;
        // A branch that returns — with a value or bare — or throws does not
        // reach the code after the `try`.
        let try_exits = try_guarantees_value_return || try_flow.guarantees_exit;
        // Reaching the code after a `try` whose handler returns or throws means
        // the block completed, so its assignments hold from there on.
        let handler_diverts = try_statement.handler.as_ref().is_none_or(|handler| {
            let flow = analyze_function_body_flow(&handler.body);
            flow.guarantees_value_return
                || flow.guarantees_exit
                || body_ends_in_never_call(&handler.body, scopes, ctx)
        });
        let mut joinable_assignments = Vec::new();
        if handler_diverts && !try_guarantees_value_return {
            branch_assigned_names(&try_statement.block, &mut joinable_assignments);
        }
        let try_block_for_widening = try_statement.block.clone();
        // A handler that completes is an edge of its own into the code after
        // the statement: joined with the block's, or alone when the block
        // cannot complete.
        let mut edge_assignments = Vec::new();
        if !handler_diverts {
            branch_assigned_names(&try_statement.block, &mut edge_assignments);
            if let Some(handler) = try_statement.handler.as_ref() {
                branch_assigned_names(&handler.body, &mut edge_assignments);
            }
        }
        scopes.push_child();
        flow_state.begin_branch_capture();
        check_function_body(
            try_statement.block,
            with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
            scopes,
            flow_state,
            ctx,
        );
        let mut try_delta = flow_state.finish_branch_capture();
        try_delta.continues = !try_exits;
        let try_assignment_types = branch_assignment_types(&joinable_assignments, scopes);
        let try_edge_types = branch_assignment_types(&edge_assignments, scopes);
        let mut catch_edge_types = Vec::new();
        scopes.pop_child();
        branch_deltas.push(try_delta);

        let try_block_assigned = deep_assigned_names(&[&try_block_for_widening]);
        let catch_body_for_widening = try_statement
            .handler
            .as_ref()
            .map(|handler| handler.body.clone())
            .unwrap_or_default();
        if let Some(handler_clause) = try_statement.handler {
            let catch_flow = analyze_function_body_flow(&handler_clause.body);
            let catch_exits = catch_flow.guarantees_value_return
                || catch_flow.guarantees_exit
                || body_ends_in_never_call(&handler_clause.body, scopes, ctx);
            // The handler can be entered from any point in the block.
            let before_catch = branch_assignment_types(&try_block_assigned, scopes);
            widen_assigned_bindings(&[&try_block_for_widening], scopes);
            scopes.push_child();
            if let Some(binding_name) = handler_clause.binding_name.as_ref() {
                // tsc types an unannotated catch variable `unknown` under
                // `strict` (`useUnknownInCatchVariables`), and rejects it as a
                // source for any parameter that is not `unknown`/`any`.
                let catch_type = handler_clause
                    .declared_type
                    .as_ref()
                    .map(|declared| catch_annotation_type(declared, handler_clause.declared_type_span, ctx))
                    .unwrap_or(if ctx.options.use_unknown_in_catch_variables {
                        Type::GenuineUnknown
                    } else {
                        Type::Any
                    });
                crate::infer::types::check_binding_pattern_reads(binding_name, &catch_type, ctx);
                insert_binding_name(binding_name, catch_type, scopes);
                crate::checks::function::signature::record_binding_names(binding_name, scopes, ctx);
            }
            flow_state.begin_branch_capture();
            // The catch variable is a binding of the clause, assigned when it
            // is entered: a same-named `var` hoisted out of some block is not
            // what the handler reads.
            flow_state.push_scope(std::collections::HashMap::new());
            if let Some(binding_name) = handler_clause.binding_name.as_ref() {
                let mut names = Vec::new();
                catch_binding_names(binding_name, &mut names);
                for name in names {
                    flow_state.declare_current(name, crate::flow::AssignmentState::Assigned);
                }
            }
            check_function_body(
                handler_clause.body,
                with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
                scopes,
                flow_state,
                ctx,
            );
            flow_state.pop_scope();
            let mut catch_delta = flow_state.finish_branch_capture();
            catch_delta.continues = !catch_exits;
            catch_edge_types = branch_assignment_types(&edge_assignments, scopes);
            scopes.pop_child();
            branch_deltas.push(catch_delta);
            restore_after_catch(&before_catch, &catch_body_for_widening, scopes);
        }

        merge_branch_deltas(flow_state, &branch_deltas, false);
        adopt_branch_assignments(&try_assignment_types, scopes);
        join_handler_edge(try_exits, try_edge_types, catch_edge_types, scopes);
        check_finalizer(
            try_statement.finalizer,
            &[&try_block_for_widening, &catch_body_for_widening],
            return_type,
            scopes,
            flow_state,
            ctx,
        );
    } else {
        // The same join the flow-active path performs. It is not conditional on
        // definite-assignment tracking: reaching the code after a `try` whose
        // handler diverts means the block completed, so what it narrowed holds
        // — and a body with no tracked locals still narrows (`context.response
        // = await fetch(…)` inside `try`, read after it, in ofetch).
        let handler_diverts = try_statement.handler.as_ref().is_none_or(|handler| {
            let flow = analyze_function_body_flow(&handler.body);
            flow.guarantees_value_return
                || flow.guarantees_exit
                || body_ends_in_never_call(&handler.body, scopes, ctx)
        });
        let mut joinable_assignments = Vec::new();
        if handler_diverts
            && !analyze_function_body_flow(&try_statement.block).guarantees_value_return
        {
            branch_assigned_names(&try_statement.block, &mut joinable_assignments);
        }
        let try_exits = {
            let flow = analyze_function_body_flow(&try_statement.block);
            flow.guarantees_value_return || flow.guarantees_exit
        };
        let mut edge_assignments = Vec::new();
        if !handler_diverts {
            branch_assigned_names(&try_statement.block, &mut edge_assignments);
            if let Some(handler) = try_statement.handler.as_ref() {
                branch_assigned_names(&handler.body, &mut edge_assignments);
            }
        }

        let try_block_for_widening = try_statement.block.clone();
        let catch_body_for_widening = try_statement
            .handler
            .as_ref()
            .map(|handler| handler.body.clone())
            .unwrap_or_default();
        let try_block_assigned = deep_assigned_names(&[&try_block_for_widening]);
        scopes.push_child();
        check_function_body(
            try_statement.block,
            with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
            scopes,
            flow_state,
            ctx,
        );
        let try_assignment_types = branch_assignment_types(&joinable_assignments, scopes);
        let try_edge_types = branch_assignment_types(&edge_assignments, scopes);
        let mut catch_edge_types = Vec::new();
        scopes.pop_child();
        adopt_branch_assignments(&try_assignment_types, scopes);

        if let Some(handler_clause) = try_statement.handler {
            // The handler can be entered from any point in the block.
            let before_catch = branch_assignment_types(&try_block_assigned, scopes);
            widen_assigned_bindings(&[&try_block_for_widening], scopes);
            scopes.push_child();
            if let Some(binding_name) = handler_clause.binding_name.as_ref() {
                // tsc types an unannotated catch variable `unknown` under
                // `strict` (`useUnknownInCatchVariables`), and rejects it as a
                // source for any parameter that is not `unknown`/`any`.
                let catch_type = handler_clause
                    .declared_type
                    .as_ref()
                    .map(|declared| catch_annotation_type(declared, handler_clause.declared_type_span, ctx))
                    .unwrap_or(if ctx.options.use_unknown_in_catch_variables {
                        Type::GenuineUnknown
                    } else {
                        Type::Any
                    });
                crate::infer::types::check_binding_pattern_reads(binding_name, &catch_type, ctx);
                insert_binding_name(binding_name, catch_type, scopes);
                crate::checks::function::signature::record_binding_names(binding_name, scopes, ctx);
            }
            check_function_body(
                handler_clause.body,
                with_type_copy_reason(TypeCopyReason::ReturnChecking, || return_type.clone()),
                scopes,
                flow_state,
                ctx,
            );
            catch_edge_types = branch_assignment_types(&edge_assignments, scopes);
            scopes.pop_child();
            restore_after_catch(&before_catch, &catch_body_for_widening, scopes);
        }
        join_handler_edge(try_exits, try_edge_types, catch_edge_types, scopes);

        check_finalizer(
            try_statement.finalizer,
            &[&try_block_for_widening, &catch_body_for_widening],
            return_type,
            scopes,
            flow_state,
            ctx,
        );
    }
}

/// The code after a `try` whose handler completes: reached from the handler
/// alone when the block cannot complete, otherwise from both.
fn join_handler_edge(
    try_exits: bool,
    try_edge_types: Vec<(String, Type)>,
    catch_edge_types: Vec<(String, Type)>,
    scopes: &mut ScopeStack,
) {
    if catch_edge_types.is_empty() {
        return;
    }
    if try_exits {
        adopt_branch_assignments(&catch_edge_types, scopes);
    } else {
        join_branch_edges(&[try_edge_types, catch_edge_types], scopes);
    }
}

pub(crate) fn check_function_throw_statement(
    throw_statement: surge_ts_syntax::ParsedThrowStatement,
    statement_index: usize,
    scopes: &ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    if flow_state.tracked_local_count() > 0 {
        let _ = check_expression_flow(
            &throw_statement.expression,
            throw_statement.expression_span,
            flow_state,
            statement_index,
            ctx,
        );
    }

    let visible_symbols = visible_symbols(scopes);
    let _ = evaluate_expression(
        &throw_statement.expression,
        throw_statement.expression_span,
        &visible_symbols,
        ctx,
    );
}

/// tsc's `checkTryStatement` and its catch variable's type: the annotation
/// must resolve to `any` or `unknown` (an alias of either is fine), TS1196 on
/// it otherwise, and a rejected annotation types the variable as the error
/// type rather than as what it names.
fn catch_annotation_type(
    declared: &surge_ts_syntax::ParsedType,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) -> Type {
    let ty = map_parsed_type(declared.clone(), ctx);
    if matches!(ty.peeled(), Type::Any | Type::GenuineUnknown | Type::ErrorType)
        || ty.peeled().is_unmodelled()
    {
        return ty;
    }
    let mut diagnostic = Diagnostic::ts1196(ctx.file_name.clone());
    if let Some(span) = span {
        diagnostic = diagnostic.with_span(convert_span(span));
    }
    ctx.push(diagnostic);
    Type::ErrorType
}

fn catch_binding_names(binding: &surge_ts_syntax::ParsedBindingName, names: &mut Vec<String>) {
    match binding {
        surge_ts_syntax::ParsedBindingName::Identifier { name, .. } => names.push(name.clone()),
        surge_ts_syntax::ParsedBindingName::ObjectPattern(pattern) => {
            for element in &pattern.elements {
                catch_binding_names(&element.binding_name, names);
            }
            if let Some(rest) = &pattern.rest {
                catch_binding_names(rest, names);
            }
        }
        surge_ts_syntax::ParsedBindingName::ArrayPattern(pattern) => {
            for element in pattern.elements.iter().flatten() {
                catch_binding_names(element, names);
            }
            if let Some(rest) = &pattern.rest {
                catch_binding_names(rest, names);
            }
        }
        surge_ts_syntax::ParsedBindingName::Unsupported { .. } => {}
    }
}
