//! Per-statement checkers for function bodies (declarations, control flow,
//! assignments, returns) dispatched from [`super::check_function_body_statement`].

use super::*;

use surge_ts_syntax::{
    ParsedExpression, ParsedFunctionBodyStatement, ParsedVariableDeclaration, ParsedVariableKind,
};
use surge_ts_types::{Type, is_assignable_to};

use crate::checks::expr::evaluate_expression;
use crate::checks::var::{VariableCheckOptions, check_variable_declaration_against_symbols};
use crate::context::CheckerContext;
use crate::flow::{
    FlowCheck, FunctionFlowState, apply_variable_declaration_state, check_expression_flow,
};
use crate::infer::InferredExpression;
use crate::symbols::{ScopeStack, SymbolInfo, SymbolKind, SymbolTable};

mod alias_conditions;
mod assignments;
mod branch_assignments;
mod control_flow;
pub(crate) mod evolving_arrays;
mod returns;
mod yields;

pub(crate) use alias_conditions::*;
pub(crate) use branch_assignments::deep_assigned_names;
pub(crate) use assignments::*;
pub(crate) use evolving_arrays::{apply_array_mutations, collect_array_mutations};
use evolving_arrays::{prime_loop_mutations, release_loop_mutations};
use branch_assignments::*;
pub(crate) use branch_assignments::branch_assigned_names;
pub(crate) use control_flow::*;
pub(crate) use returns::*;
pub(crate) use yields::{
    GeneratorYields, collecting_generator_yields, implicit_any_yield_starts, record_generator_yield,
    yield_operands, yields_only_widening_nullish,
};

pub(crate) fn check_function_variable_declaration(
    variable: ParsedVariableDeclaration,
    statement_index: usize,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let local_name = variable.name.clone();
    let variable_kind = variable.kind;
    // An annotation naming an in-scope type parameter resolves to the body's
    // placeholder, which reads as degraded; definite assignment needs to know
    // it is `T` (see `type_assumed_initialized`).
    let annotated_type_parameter = match &variable.declared_type {
        Some(surge_ts_syntax::ParsedType::Named(named))
            if named.type_arguments.is_empty() && ctx.type_parameter_in_scope(&named.name) =>
        {
            Some(Type::TypeParameter(surge_ts_types::TypeParameterType {
                name: named.name.as_str().into(),
                owner: 0,
            }))
        }
        _ => None,
    };
    let has_initializer = variable.initializer.is_some();
    let contextual_annotation = variable.declared_type.is_some() && annotated_type_parameter.is_none();
    // An ambient (`declare`) binding never carries an initializer but is not
    // "unassigned" — it is provided from outside. Body-local `enum`s lower to
    // one, so definite-assignment analysis must not report TS2454 on them. A
    // `let x!: T` definite-assignment assertion says the same thing explicitly.
    let definitely_assigned =
        has_initializer || variable.is_declare || variable.has_definite_assertion;

    // Track a boolean alias of a guard expression (`const ok = error &&
    // isError(error) && …`) so a later `if (!ok) return;` can narrow the guarded
    // identifiers in the fall-through, matching tsc's aliased-condition handling.
    if let Some(initializer) = variable.initializer.as_ref() {
        flow_state
            .record_alias_guard_targets(local_name.clone(), guarded_value_identifiers(initializer));
        // A `const` whose initializer is plainly a condition keeps that
        // condition, so a later `if (ok)` narrows exactly as the written
        // expression would (tsc's aliased-condition narrowing). tsc inlines
        // only an unannotated `const` declared on its own, not a destructured
        // element; an alias of an alias and a type-predicate call qualify too.
        let inlinable = matches!(variable_kind, ParsedVariableKind::Const)
            && variable.declared_type.is_none()
            && !variable.from_binding_pattern
            && (is_condition_shaped(initializer)
                || matches!(initializer, ParsedExpression::Identifier { name, .. }
                    if scopes.visible_symbols().alias_condition(name).is_some())
                || is_type_predicate_call(initializer, scopes, ctx));
        if inlinable {
            let condition = std::sync::Arc::new(initializer.clone());
            flow_state.record_alias_guard_condition(local_name.clone(), condition.clone());
            scopes.record_alias_condition(local_name.as_str(), Some(condition));
        } else {
            scopes.record_alias_condition(local_name.as_str(), None);
        }
        // A `const` bound to a property reference is a discriminant alias:
        // `const { direction } = opts` lowers to `direction = opts.direction`,
        // and testing `direction` narrows `opts`.
        if matches!(variable_kind, ParsedVariableKind::Const) && is_property_reference(initializer)
        {
            flow_state.record_discriminant_alias(
                local_name.clone(),
                std::sync::Arc::new(initializer.clone()),
            );
        }
        // `const [error, value] = tuple` lowers to one `tuple[N]` binding per
        // element; remembering which element each one is makes the group
        // dependent when `tuple` is a union of tuples.
        let destructured = matches!(variable_kind, ParsedVariableKind::Const)
            .then(|| {
                tuple_destructure_binding(
                    initializer,
                    variable.array_pattern_span,
                    variable.from_binding_pattern,
                    &visible_symbols(scopes),
                    ctx,
                )
            })
            .flatten();
        scopes.record_tuple_destructure(local_name.as_str(), destructured);
    }

    check_local_duplicate_declaration(&variable, scopes, ctx);
    let auto_declaration = evolving_arrays::auto_declaration(&variable, ctx);
    let auto_variable = auto_declaration.map(|_| ParsedVariableDeclaration {
        declared_type: None,
        initializer: variable
            .initializer
            .as_ref()
            .filter(|initializer| {
                matches!(
                    initializer,
                    ParsedExpression::UndefinedLiteral | ParsedExpression::NullLiteral
                )
            })
            .cloned()
            .or_else(|| variable.initializer.as_ref().map(|_| ParsedExpression::Unknown)),
        ..variable.clone()
    });

    let initializer_flow_blocked = variable.initializer.as_ref().is_some_and(|initializer| {
        let block_scoped = matches!(
            variable_kind,
            ParsedVariableKind::Let | ParsedVariableKind::Const
        );
        if flow_state.tracked_local_count() == 0 && !block_scoped {
            return false;
        }

        let circular_var_starts = ctx.circular_var_starts.clone();
        let initializing = crate::flow::InitializingBinding::for_initializer(
            &variable,
            || {
                variable
                    .declared_type
                    .as_ref()
                    .map(|annotation| crate::flow::excludes_undefined(annotation, ctx, &[]))
            },
            &circular_var_starts,
        )
        .map(|binding| flow_state.begin_initializer([binding]));

        // A written annotation other than a bare type parameter is a
        // non-generic contextual type for the initializer.
        let blocked = if contextual_annotation {
            crate::flow::check_substituting_read_flow(
                initializer,
                variable.initializer_span,
                flow_state,
                statement_index,
                ctx,
            )
        } else {
            check_expression_flow(
                initializer,
                variable.initializer_span,
                flow_state,
                statement_index,
                ctx,
            )
        }
        .is_blocked();
        if let Some(mark) = initializing {
            flow_state.end_initializer(mark);
        }
        blocked
    });

    // The declared name is visible inside its own initializer's nested function
    // bodies (`const t = setInterval(() => clearInterval(t), 10)`), which run
    // after the binding exists. It is seeded as the degradation sentinel so the
    // closure reference resolves without inventing a type; the real symbol
    // replaces it below. A *direct* self-read is still caught by the flow layer's
    // temporal-dead-zone check, which runs above. A redeclared `var` is
    // one symbol that already has its first declaration's type.
    let redeclared_var = matches!(variable_kind, ParsedVariableKind::Var)
        && visible_symbols(scopes)
            .get_own(&local_name)
            .is_some_and(|existing| matches!(existing.kind, SymbolKind::Var) && !existing.ty.is_unknown());
    if has_initializer && !redeclared_var {
        scopes.insert_current(
            local_name.as_str(),
            SymbolInfo {
                ty: Type::Unknown,
                kind: symbol_kind_for_variable(variable_kind),
                function_signature: None,
            },
        );
    }

    let has_written_annotation = variable.declared_type.is_some();
    let literal_initializer_type = matches!(
        variable_kind,
        ParsedVariableKind::Let | ParsedVariableKind::Var | ParsedVariableKind::Const
    )
    .then(|| literal_initializer_type(variable.initializer.as_ref()))
    .flatten();

    let visible_symbols = visible_symbols(scopes);

    let is_annotated = variable.declared_type.is_some();
    let probe_initializer = (literal_initializer_type.is_none()
        && variable.declared_type.is_some()
        && !initializer_flow_blocked)
        .then(|| variable.initializer.clone())
        .flatten();
    let enum_probe = (!is_annotated
        && matches!(variable_kind, ParsedVariableKind::Let | ParsedVariableKind::Var)
        && !initializer_flow_blocked)
        .then(|| variable.initializer.clone().filter(crate::checks::var::may_read_enum_member))
        .flatten();

    let variable_name_span = variable.name_span;
    if let Some(symbol) = check_variable_declaration_against_symbols(
        variable,
        visible_symbols,
        ctx,
        VariableCheckOptions {
            report_duplicate_let_const: false,
            check_initializer: !initializer_flow_blocked,
            check_type_literal: true,
        },
    ) {
        crate::semantic::record_declaration_type(variable_name_span, &symbol.ty, ctx);
        apply_variable_declaration_state(
            variable_kind,
            local_name.as_str(),
            definitely_assigned,
            Some(annotated_type_parameter.as_ref().unwrap_or(&symbol.ty)),
            flow_state,
            ctx,
        );
        if let Some(Type::TypeParameter(parameter)) = &annotated_type_parameter
            && crate::flow::constraint_substitutes(&parameter.name, ctx)
        {
            flow_state.constraint_exempt.insert(local_name.as_str().into());
        }
        // A declared union narrows to what the initializer can inhabit, exactly
        // as a later assignment does: `let style: Style = "simple"` is
        // `"simple"` until reassigned, and `let v: string | number = "a"` is
        // `string`. Recorded as a *narrowing* so a later
        // assignment still checks against the declaration. Only a literal
        // initializer participates — its type is known without re-evaluating
        // (and re-reporting) the expression.
        let narrowed = literal_initializer_type
            .filter(|initialized| {
                // `boolean` is tsc's `true | false` union.
                matches!(symbol.ty, Type::Union(_) | Type::Boolean)
                    && is_assignable_to(initialized, &symbol.ty)
            })
            .map(|initialized| assignments::assignment_reduced_type(Some(&symbol.ty), initialized))
            .or_else(|| {
                let Type::Union(union) = &symbol.ty else {
                    return None;
                };
                let initializer = probe_initializer.as_ref()?;
                let assigned = assigned_initializer_type(initializer, visible_symbols, ctx)?;
                let keep = |assigned: &Type| -> Vec<Type> {
                    union.types().iter().filter(|member| is_assignable_to(assigned, member)).cloned().collect()
                };
                let mut kept = keep(&assigned);
                // A generic call infers from the declared type as its contextual
                // return type (`let d: Deferred<T> | null = createDeferred()`).
                if kept.is_empty()
                    && let Some(contextual) = contextual_call_initializer_type(initializer, &symbol.ty, visible_symbols, ctx)
                {
                    kept = keep(&contextual);
                }
                (!kept.is_empty() && kept.len() < union.types().len())
                    .then(|| surge_ts_types::union_type(kept))
            })
            .or_else(|| {
                crate::checks::var::enum_member_initializer_narrowing(
                    enum_probe.as_ref()?,
                    &symbol.ty,
                    visible_symbols,
                    ctx,
                )
            });
        match narrowed {
            Some(initialized) => {
                let declared = symbol.ty.clone();
                scopes.insert_current_handle(local_name.as_str(), symbol);
                scopes.insert_current_declared(
                    local_name.as_str(),
                    SymbolInfo {
                        ty: initialized,
                        kind: symbol_kind_for_variable(variable_kind),
                        function_signature: None,
                    },
                    declared,
                );
            }
            // An annotation is the binding's declared type for good: recording
            // it lets a later write be checked against it and narrowed from
            // it, rather than rewriting the binding to whatever was assigned.
            None if is_annotated && !symbol.ty.is_unknown() => {
                let declared = symbol.ty.clone();
                let info = SymbolInfo {
                    ty: declared.clone(),
                    kind: symbol_kind_for_variable(variable_kind),
                    function_signature: symbol.function_signature.clone(),
                };
                scopes.insert_current_handle(local_name.as_str(), symbol);
                scopes.insert_current_declared(local_name.as_str(), info, declared);
            }
            None => {
                // An un-annotated `let`/`var` is declared at its initializer's
                // *widened* type (`getWidenedTypeForVariableLikeDeclaration`),
                // and a later assignment narrows within that — `let status =
                // state.status` stays the union a written `"success"` selects
                // from instead of widening to `string`.
                // Only a *union* declaration is recorded: that is the shape a
                // later assignment narrows within (`assignment_reduced_type`).
                // An `undefined`/auto initializer must stay unbound — it is
                // still evolving, and binding it would reject the assignments
                // that give it its type.
                let declared = (!has_written_annotation
                    && matches!(variable_kind, ParsedVariableKind::Let | ParsedVariableKind::Var)
                    && matches!(symbol.ty, Type::Union(_)))
                .then(|| symbol.ty.clone());
                match declared {
                    Some(declared) => {
                        scopes.insert_current_narrowed(
                            local_name.as_str(),
                            SymbolInfo {
                                ty: declared.clone(),
                                kind: symbol_kind_for_variable(variable_kind),
                                function_signature: symbol.function_signature.clone(),
                            },
                            declared,
                        );
                    }
                    None => {
                        scopes.insert_current_handle(local_name.as_str(), symbol);
                    }
                }
            }
        }
        match &auto_variable {
            Some(declaration) => evolving_arrays::declare_auto_binding(
                declaration,
                auto_declaration,
                scopes,
                ctx,
            ),
            None => scopes.declare_auto_array(local_name.as_str(), None),
        }
    }
}

/// tsc narrows an annotated union declaration by whatever it is initialized
/// with, not only by a literal: `let client: PC | undefined = persisted` starts
/// out as `PC`. Only a cleanly inferred initializer proves anything — a written
/// `unknown` is not a degradation, so a type that carries one (`{ [k: string]:
/// unknown }`) still narrows. The initializer was already checked against the
/// annotation; this re-evaluation runs without that contextual type, so
/// whatever it reports (an implicit-any method parameter, say) is a probe
/// artifact and is discarded.
fn assigned_initializer_type(
    initializer: &ParsedExpression,
    visible_symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let checkpoint = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(initializer, visible_symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    // Shallow on purpose: a nominal reference to an imported interface is a
    // clean answer even when a member deep inside it resolved to the sentinel,
    // and the deep walk refused every `let client: PersistedClient | undefined
    // = persistedClient` in the persister packages, leaving the binding at its
    // annotation.
    match inferred {
        InferredExpression::Known(ty)
            if !ty.is_unknown()
                && !matches!(ty, Type::Any)
                && !initializer_type_is_degraded(&ty) =>
        {
            Some(ty)
        }
        _ => None,
    }
}

fn contextual_call_initializer_type(
    initializer: &ParsedExpression,
    declared: &Type,
    visible_symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let ParsedExpression::Call { callee_name, callee_span, type_arguments, arguments, .. } = initializer else {
        return None;
    };
    if !type_arguments.is_empty() {
        return None;
    }
    let checkpoint = ctx.diagnostics().len();
    let returned = crate::checks::call::check_call_like_with_expected_type(
        callee_name,
        *callee_span,
        None,
        type_arguments,
        arguments,
        Some(declared),
        visible_symbols,
        ctx,
    );
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    returned.filter(|ty| !ty.is_unknown() && !matches!(ty, Type::Any) && !initializer_type_is_degraded(ty))
}

fn initializer_type_is_degraded(ty: &Type) -> bool {
    match ty {
        Type::Unknown | Type::TypeParameter(_) => true,
        Type::Union(union) => union.types().iter().any(initializer_type_is_degraded),
        _ => false,
    }
}

fn symbol_kind_for_variable(kind: ParsedVariableKind) -> SymbolKind {
    match kind {
        ParsedVariableKind::Let => SymbolKind::Let,
        ParsedVariableKind::Const => SymbolKind::Const,
        ParsedVariableKind::Var => SymbolKind::Var,
    }
}

/// The type of a literal initializer, which needs no expression evaluation (and
/// so cannot double-report the initializer's own diagnostics).
fn literal_initializer_type(initializer: Option<&ParsedExpression>) -> Option<Type> {
    match initializer? {
        ParsedExpression::StringLiteral(value) => Some(Type::StringLiteral(value.clone())),
        ParsedExpression::NumberLiteral(value) => {
            Some(Type::NumberLiteral(surge_ts_types::NumberLiteralType {
                value: value.clone(),
            }))
        }
        ParsedExpression::BooleanLiteral(value) => Some(Type::BooleanLiteral(*value)),
        _ => None,
    }
}

pub(crate) fn check_function_block(
    block_body: Vec<ParsedFunctionBodyStatement>,
    return_type: Option<&Type>,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    // A bare block always runs, so what it assigns to an outer binding holds
    // after it; a binding the block declares itself is not the outer one.
    let mut assigned = Vec::new();
    branch_assigned_names(&block_body, &mut assigned);
    assigned.retain(|name| {
        !block_body.iter().any(|statement| {
            matches!(
                statement,
                ParsedFunctionBodyStatement::VariableDeclaration(variable) if variable.name == *name
            )
        })
    });
    scopes.push_child();
    check_function_body(block_body, return_type, scopes, flow_state, ctx);
    let assigned_types = branch_assignment_types(&assigned, scopes);
    scopes.pop_child();
    adopt_branch_assignments(&assigned_types, scopes);
}

pub(crate) fn check_function_expression_statement(
    expression: ParsedExpression,
    statement_index: usize,
    scopes: &mut ScopeStack,
    flow_state: &mut FunctionFlowState,
    ctx: &mut CheckerContext,
) {
    let flow_blocked = if flow_state.tracked_local_count() > 0 {
        check_expression_flow(&expression, None, flow_state, statement_index, ctx)
    } else {
        FlowCheck::Clear
    };

    if flow_blocked.is_blocked() {
        return;
    }

    let visible_symbols = visible_symbols(scopes);
    let _ = evaluate_expression(&expression, None, &visible_symbols, ctx);

    // `assertIsObject(obj);` narrows from here to the end of the block, not
    // inside a branch, so the narrowing is applied at the statement.
    crate::checks::function::narrow_assertion_call_in_scope(&expression, scopes, ctx);
}
