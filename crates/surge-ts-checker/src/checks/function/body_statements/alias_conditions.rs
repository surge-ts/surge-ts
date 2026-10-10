
use surge_ts_syntax::{
    ParsedExpression,
    ParsedFunctionBodyStatement,
    ParsedUnaryOperator,
};
use surge_ts_types::Type;

use crate::context::CheckerContext;
use crate::flow::FunctionFlowState;
use crate::infer::InferredExpression;
use crate::symbols::{DestructureKey, ScopeStack, SymbolTable, TupleDestructureBinding};
use super::super::{downgrade_genuine_unknown_in_scope, narrow_discriminant_in_scope};

/// Whether an initializer is plainly a boolean condition — a logical chain, a
/// negation, or a comparison. Deliberately narrow: a `const x = f()` is not
/// recorded, so the aliased-condition clone stays proportional to guard
/// aliases rather than to every `const` in the body.
pub(crate) fn is_condition_shaped(expression: &ParsedExpression) -> bool {
    use surge_ts_syntax::{ParsedBinaryOperator, ParsedUnaryOperator};
    match expression {
        ParsedExpression::Logical { .. } => true,
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            ..
        } => true,
        ParsedExpression::Binary { operator, .. } => matches!(
            operator,
            ParsedBinaryOperator::StrictEquals
                | ParsedBinaryOperator::Equals
                | ParsedBinaryOperator::StrictNotEquals
                | ParsedBinaryOperator::NotEquals
                | ParsedBinaryOperator::Instanceof
        ),
        _ => false,
    }
}

/// The tuple element an array-destructured binding was lowered from: `const
/// [a, b] = xs` lowers each binding to `xs[0]`, `xs[1]`, and a pattern over any
/// other expression to an element access on that expression. Only a source
/// that is a union of tuples makes the bindings dependent, so nothing else is
/// recorded for the expression form.
pub(super) fn tuple_destructure_binding(
    expression: &ParsedExpression,
    pattern_span: Option<surge_ts_syntax::TextSpan>,
    from_binding_pattern: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<TupleDestructureBinding> {
    let literal_index = |index: &ParsedExpression| match index {
        ParsedExpression::NumberLiteral(value) => value.parse::<usize>().ok(),
        _ => None,
    };
    match expression {
        ParsedExpression::IndexAccess {
            object_name, index, ..
        } => Some(TupleDestructureBinding {
            source: object_name.as_str().into(),
            key: DestructureKey::Index(literal_index(index)?),
            source_type: None,
        }),
        // `const { kind, payload } = action` lowers each binding to a property
        // read. One written on its own (`const kind = action.kind`) has no
        // siblings and is the discriminant-alias case instead.
        ParsedExpression::PropertyAccess {
            object,
            object_span,
            property_name,
            ..
        } if from_binding_pattern => match object.as_ref() {
            ParsedExpression::Identifier { name, .. } => Some(TupleDestructureBinding {
                source: name.as_str().into(),
                key: DestructureKey::Property(property_name.as_str().into()),
                source_type: None,
            }),
            source => {
                let object_span = (*object_span)?;
                let InferredExpression::Known(source_type) =
                    crate::infer::infer_expression(source, symbols, ctx)
                else {
                    return None;
                };
                matches!(source_type.peeled(), Type::Union(_)).then(|| TupleDestructureBinding {
                    source: format!("\0pattern@{}", object_span.start).into(),
                    key: DestructureKey::Property(property_name.as_str().into()),
                    source_type: Some(source_type),
                })
            }
        },
        ParsedExpression::ElementAccess { object, index, .. } => {
            let index = literal_index(index)?;
            let pattern_span = pattern_span?;
            let InferredExpression::Known(source_type) =
                crate::infer::infer_expression(object, symbols, ctx)
            else {
                return None;
            };
            let Type::Union(union) = source_type.peeled() else {
                return None;
            };
            if !union
                .types()
                .iter()
                .all(|member| matches!(member.peeled(), Type::Tuple(_)))
            {
                return None;
            }
            Some(TupleDestructureBinding {
                source: format!("\0pattern@{}", pattern_span.start).into(),
                key: DestructureKey::Index(index),
                source_type: Some(source_type),
            })
        }
        _ => None,
    }
}

/// Applies the destructured-discriminated-union narrowing to the current scope
/// (see [`tuple_destructure_sibling_narrowings`]).
pub(super) fn narrow_tuple_destructure_siblings(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
) {
    let narrowings = crate::checks::function::narrowing::tuple_destructure_sibling_narrowings(
        condition,
        scopes.visible_symbols(),
        branch_is_true,
    );
    for (name, symbol, declared) in narrowings {
        let _ = scopes.insert_current_narrowed(name.to_string(), symbol, declared);
    }
}

/// Narrows by a condition and, when it named a discriminant alias, by the
/// rewritten form as well. Both are applied: the written condition narrows the
/// alias binding itself (`if (transformer)` proves the local non-nullish), the
/// rewrite narrows the object it came from (`opts.transformer`).
pub(super) fn narrow_condition_and_aliases_in_scope(
    base: &ParsedExpression,
    rewritten: Option<&ParsedExpression>,
    alias_source: Option<&ParsedExpression>,
    scopes: &mut ScopeStack,
    branch_is_true: bool,
    ctx: &mut CheckerContext,
) {
    narrow_discriminant_in_scope(base, scopes, branch_is_true, ctx);
    if let Some(rewritten) = rewritten {
        narrow_discriminant_in_scope(rewritten, scopes, branch_is_true, ctx);
    }
    // A `const` alias of a condition is narrowed by the test as well as the
    // expression it was written as: tsc narrows both `s` and what
    // `const s = a[i] || rest` reads. Expanding the alias alone left `s` at its
    // declared type inside the branch.
    if let Some(alias_source) = alias_source {
        narrow_discriminant_in_scope(alias_source, scopes, branch_is_true, ctx);
    }
    narrow_tuple_destructure_siblings(base, scopes, branch_is_true);
}

/// Whether an initializer is a static property reference over identifiers or
/// `this` (`opts.direction`, `node.kind.value`, `this.test.type`) — the shape a discriminant alias takes.
pub(super) fn is_property_reference(expression: &ParsedExpression) -> bool {
    match expression {
        ParsedExpression::PropertyAccess { object, .. }
        | ParsedExpression::OptionalPropertyAccess { object, .. } => {
            matches!(
                object.as_ref(),
                ParsedExpression::Identifier { .. } | ParsedExpression::This { .. }
            ) || is_property_reference(object)
        }
        _ => false,
    }
}

/// Substitutes discriminant aliases for the references they were bound to, so
/// `if (direction === "up")` narrows `opts` exactly as `opts.direction === "up"`
/// would. `None` when the condition names no alias.
pub(super) fn rewrite_discriminant_aliases(
    condition: &ParsedExpression,
    flow_state: &FunctionFlowState,
) -> Option<ParsedExpression> {
    match condition {
        ParsedExpression::Identifier { name, .. } => flow_state.discriminant_alias(name).cloned(),
        ParsedExpression::Unary {
            operator,
            operator_span,
            operand,
            operand_span,
        } => {
            let rewritten = rewrite_discriminant_aliases(operand, flow_state)?;
            Some(ParsedExpression::Unary {
                operator: *operator,
                operator_span: *operator_span,
                operand: Box::new(rewritten),
                operand_span: *operand_span,
            })
        }
        ParsedExpression::Binary {
            left,
            left_span,
            operator,
            operator_span,
            right,
            right_span,
            compound,
        } => {
            let new_left = rewrite_discriminant_aliases(left, flow_state);
            let new_right = rewrite_discriminant_aliases(right, flow_state);
            if new_left.is_none() && new_right.is_none() {
                return None;
            }
            Some(ParsedExpression::Binary {
                left: Box::new(new_left.unwrap_or_else(|| left.as_ref().clone())),
                left_span: *left_span,
                operator: *operator,
                operator_span: *operator_span,
                right: Box::new(new_right.unwrap_or_else(|| right.as_ref().clone())),
                right_span: *right_span,
                compound: *compound,
            })
        }
        ParsedExpression::Logical {
            left,
            left_span,
            operator,
            operator_span,
            right,
            right_span,
            ..
        } => {
            let new_left = rewrite_discriminant_aliases(left, flow_state);
            let new_right = rewrite_discriminant_aliases(right, flow_state);
            if new_left.is_none() && new_right.is_none() {
                return None;
            }
            Some(ParsedExpression::Logical {
                left: Box::new(new_left.unwrap_or_else(|| left.as_ref().clone())),
                left_span: *left_span,
                operator: *operator,
                operator_span: *operator_span,
                right: Box::new(new_right.unwrap_or_else(|| right.as_ref().clone())),
                right_span: *right_span,
                truthiness_tests: Vec::new(),
            })
        }
        _ => None,
    }
}

/// The condition an `if` really tests: an identifier bound to a boolean `const`
/// guard stands for the expression it was initialized from, also when it is one
/// operand of the condition's `&&`/`||`/`!` structure (`isRefetch && mode ===
/// 'reset'` narrows through `isRefetch`). An alias within the inlined
/// condition is inlined in turn, up to tsc's five levels, and each inlined
/// condition keeps only the guards over constant references.
pub(super) fn resolved_alias_condition(
    condition: &ParsedExpression,
    scopes: &ScopeStack,
    flow_state: &FunctionFlowState,
) -> Option<std::sync::Arc<ParsedExpression>> {
    expand_alias_conditions(condition, scopes, flow_state, 0).map(std::sync::Arc::new)
}

fn expand_alias_conditions(
    condition: &ParsedExpression,
    scopes: &ScopeStack,
    flow_state: &FunctionFlowState,
    inline_level: usize,
) -> Option<ParsedExpression> {
    match condition {
        ParsedExpression::Identifier { name, .. } => {
            if inline_level >= crate::checks::function::ALIAS_INLINE_LIMIT {
                return None;
            }
            let alias = flow_state.alias_guard_condition(name)?;
            let inlined = expand_alias_conditions(&alias, scopes, flow_state, inline_level + 1)
                .unwrap_or_else(|| (*alias).clone());
            Some(crate::checks::function::retain_constant_reference_guards(
                &inlined,
                scopes.visible_symbols(),
                &|name| flow_state.is_binding_assigned(name),
            ))
        }
        ParsedExpression::Logical {
            left,
            left_span,
            operator,
            operator_span,
            right,
            right_span,
            ..
        } => {
            let expanded_left = expand_alias_conditions(left, scopes, flow_state, inline_level);
            let expanded_right = expand_alias_conditions(right, scopes, flow_state, inline_level);
            if expanded_left.is_none() && expanded_right.is_none() {
                return None;
            }
            Some(ParsedExpression::Logical {
                left: Box::new(expanded_left.unwrap_or_else(|| (**left).clone())),
                left_span: *left_span,
                operator: *operator,
                operator_span: *operator_span,
                right: Box::new(expanded_right.unwrap_or_else(|| (**right).clone())),
                right_span: *right_span,
                truthiness_tests: Vec::new(),
            })
        }
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operator_span,
            operand,
            operand_span,
        } => expand_alias_conditions(operand, scopes, flow_state, inline_level).map(|expanded| {
            ParsedExpression::Unary {
                operator: ParsedUnaryOperator::Not,
                operator_span: *operator_span,
                operand: Box::new(expanded),
                operand_span: *operand_span,
            }
        }),
        _ => None,
    }
}

/// Whether a branch body ends in a call to a `never`-returning function
/// (`process.exit(1)`), which ends control flow exactly as a `return` does.
/// [`analyze_function_body_flow`] is purely syntactic and cannot see a return
/// type, so this one type-dependent case is decided here, where the scope is in
/// hand — without it the fall-through of `if (!args.file) { …; process.exit(1); }`
/// keeps the unnarrowed `string | undefined`.
pub(crate) fn body_ends_in_never_call(
    body: &[ParsedFunctionBodyStatement],
    scopes: &ScopeStack,
    ctx: &CheckerContext,
) -> bool {
    match body.last() {
        Some(ParsedFunctionBodyStatement::Block(block)) => body_ends_in_never_call(block, scopes, ctx),
        Some(ParsedFunctionBodyStatement::Expression(expression)) => {
            call_returns_never(expression, scopes, ctx)
        }
        _ => false,
    }
}

/// The call statements of `body` (nested blocks included, nested functions
/// not) whose callee is declared to return `never`, by
/// [`crate::flow::call_statement_key`].
pub(crate) fn never_call_statements(
    body: &[ParsedFunctionBodyStatement],
    scopes: &ScopeStack,
    ctx: &CheckerContext,
) -> Vec<(usize, usize)> {
    let mut keys = Vec::new();
    collect_never_call_statements(body, scopes, ctx, &mut keys);
    keys
}

fn collect_never_call_statements(
    body: &[ParsedFunctionBodyStatement],
    scopes: &ScopeStack,
    ctx: &CheckerContext,
    keys: &mut Vec<(usize, usize)>,
) {
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::Expression(expression) => {
                if call_returns_never(expression, scopes, ctx)
                    && let Some(key) = crate::flow::call_statement_key(expression)
                {
                    keys.push(key);
                }
            }
            ParsedFunctionBodyStatement::Block(statements) => collect_never_call_statements(statements, scopes, ctx, keys),
            ParsedFunctionBodyStatement::If(statement) => {
                collect_never_call_statements(&statement.then_body, scopes, ctx, keys);
                collect_never_call_statements(&statement.else_body, scopes, ctx, keys);
            }
            ParsedFunctionBodyStatement::While(statement) => collect_never_call_statements(&statement.body, scopes, ctx, keys),
            ParsedFunctionBodyStatement::ForOf(statement) => collect_never_call_statements(&statement.body, scopes, ctx, keys),
            ParsedFunctionBodyStatement::Switch(statement) => {
                for case in &statement.cases {
                    collect_never_call_statements(&case.consequent, scopes, ctx, keys);
                }
            }
            ParsedFunctionBodyStatement::Try(statement) => {
                collect_never_call_statements(&statement.block, scopes, ctx, keys);
                if let Some(handler) = &statement.handler {
                    collect_never_call_statements(&handler.body, scopes, ctx, keys);
                }
                collect_never_call_statements(&statement.finalizer, scopes, ctx, keys);
            }
            _ => {}
        }
    }
}

/// Whether a call expression's callee is declared to return `never`.
pub(super) fn call_returns_never(expression: &ParsedExpression, scopes: &ScopeStack, ctx: &CheckerContext) -> bool {
    let returns_never = |ty: &Type| {
        matches!(ty, Type::Function(function) if matches!(function.return_type(), Type::Never))
    };
    match expression {
        ParsedExpression::Call { callee_name, .. } => scopes
            .resolve(callee_name)
            .is_some_and(|symbol| returns_never(&symbol.ty)),
        ParsedExpression::PropertyCall {
            object,
            property_name,
            ..
        } => {
            let name = match object.as_ref() {
                ParsedExpression::Identifier { name, .. } => name.as_str(),
                ParsedExpression::This { .. } => "this",
                _ => return false,
            };
            scopes.resolve(name).is_some_and(|symbol| {
                symbol
                    .ty
                    .get_property_access_type(property_name)
                    .is_some_and(|ty| returns_never(&ty))
            })
                // A namespace's function member is bound under its qualified
                // name, outside the body's scopes.
                || scopes.resolve(name).is_none()
                    && crate::infer::expression::qualified_namespace_member(&format!("{name}.{property_name}"), &ctx.symbols, ctx)
                        .is_some_and(|symbol| returns_never(&symbol.ty))
        }
        _ => false,
    }
}

/// `if (!ok) <exit>` where `ok` is a boolean alias of a guard expression
/// narrows, in the fall-through, the identifiers that alias guarded — dropping a
/// guarded genuine-`unknown` to the degradation sentinel so a later access is
/// not a spurious `TS18046`. Mirrors tsc's aliased-condition narrowing, limited
/// to the genuine-unknown downgrade.
pub(super) fn narrow_aliased_guard_after_exit(
    condition: &ParsedExpression,
    scopes: &mut ScopeStack,
    flow_state: &FunctionFlowState,
) {
    let ParsedExpression::Unary {
        operator: ParsedUnaryOperator::Not,
        operand,
        ..
    } = condition
    else {
        return;
    };
    let ParsedExpression::Identifier { name, .. } = operand.as_ref() else {
        return;
    };
    if let Some(targets) = flow_state.alias_guard_targets(name) {
        let targets = targets.to_vec();
        downgrade_genuine_unknown_in_scope(&targets, scopes);
    }
}
