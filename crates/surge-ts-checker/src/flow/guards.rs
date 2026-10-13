//! What a condition proves about an unassigned local on each of its edges.
//!
//! tsc does not track assignment separately from narrowing: an unassigned
//! local reads as `declared | undefined` (`getFlowTypeOfReference` with an
//! `undefined` initial type), and `checkIdentifier` reports TS2454 only when
//! that flow type still contains `undefined`. A guard that strips `undefined`
//! on an edge — `typeof x === "string"`, `x instanceof C`, truthiness, an
//! equality with a value that is never `undefined` — leaves nothing to report
//! past it. The binder also folds the `true`/`false` keywords out of the flow
//! graph (`createFlowCondition`), so the edge a literal condition never takes
//! is unreachable, and an unreachable reference reads as its declared type.

use surge_ts_syntax::{
    ParsedBinaryOperator, ParsedExpression, ParsedLogicalOperator, ParsedUnaryOperator,
};

/// The locals `condition` proves are not `undefined` when it evaluates to
/// `when`. `predicate_parameter` answers which argument a callee's
/// `param is T` predicate tests.
pub(crate) fn condition_defined_names<'e>(
    condition: &'e ParsedExpression,
    when: bool,
    predicate_parameter: &dyn Fn(&str) -> Option<usize>,
) -> Vec<&'e str> {
    condition_defined_names_with(condition, when, predicate_parameter, &|_, _| false)
}

/// [`condition_defined_names`], where `never_undefined` answers whether a
/// local compared with another (`x === y`) is typed without `undefined` (and
/// without `null`, for a loose comparison): `narrowTypeByEquality` keeps of
/// the other only what is comparable to it.
pub(crate) fn condition_defined_names_with<'e>(
    condition: &'e ParsedExpression,
    when: bool,
    predicate_parameter: &dyn Fn(&str) -> Option<usize>,
    never_undefined: &dyn Fn(&str, bool) -> bool,
) -> Vec<&'e str> {
    let mut names = Vec::new();
    collect(condition, when, &Guards { predicate_parameter, never_undefined }, &mut names);
    names
}

struct Guards<'a> {
    predicate_parameter: &'a dyn Fn(&str) -> Option<usize>,
    never_undefined: &'a dyn Fn(&str, bool) -> bool,
}

/// Whether `condition` never takes its `when` edge: the binder folds the
/// `true`/`false` keywords out of the flow graph (`createFlowCondition`), and
/// the fold carries through `!`, `&&` and `||` (`bindCondition`).
pub(crate) fn condition_never_takes(condition: &ParsedExpression, when: bool) -> bool {
    !can_take(condition, when)
}

fn can_take(condition: &ParsedExpression, when: bool) -> bool {
    match condition {
        ParsedExpression::BooleanLiteral(value) => *value == when,
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => can_take(operand, !when),
        ParsedExpression::Logical {
            operator,
            left,
            right,
            ..
        } => {
            // The operator's own edge takes the left operand's; the other one
            // is the right operand's, reached through the left's.
            let short_circuit = matches!(operator, ParsedLogicalOperator::Or);
            if when == short_circuit {
                can_take(left, when) || (can_take(left, !when) && can_take(right, when))
            } else {
                can_take(left, when) && can_take(right, when)
            }
        }
        _ => true,
    }
}

fn collect<'e>(
    expression: &'e ParsedExpression,
    when: bool,
    guards: &Guards<'_>,
    out: &mut Vec<&'e str>,
) {
    match expression {
        ParsedExpression::Identifier { name, .. } => {
            if when {
                out.push(name);
            }
        }
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Not,
            operand,
            ..
        } => collect(operand, !when, guards, out),
        ParsedExpression::Logical {
            operator,
            left,
            right,
            ..
        } => {
            // `a && b` is true only when both are; `a || b` is false only when
            // both are. The other edge holds only what both operands prove.
            let both = matches!(
                (operator, when),
                (ParsedLogicalOperator::And, true) | (ParsedLogicalOperator::Or, false)
            );
            if both {
                collect(left, when, guards, out);
                collect(right, when, guards, out);
            } else {
                let mut from_left = Vec::new();
                collect(left, when, guards, &mut from_left);
                let mut from_right = Vec::new();
                collect(right, when, guards, &mut from_right);
                out.extend(from_left.into_iter().filter(|name| from_right.contains(name)));
            }
        }
        ParsedExpression::Binary {
            left,
            operator,
            right,
            ..
        } => match operator {
            ParsedBinaryOperator::StrictEquals
            | ParsedBinaryOperator::Equals
            | ParsedBinaryOperator::StrictNotEquals
            | ParsedBinaryOperator::NotEquals => {
                let equal_edge = matches!(
                    operator,
                    ParsedBinaryOperator::StrictEquals | ParsedBinaryOperator::Equals
                ) == when;
                let strict = matches!(
                    operator,
                    ParsedBinaryOperator::StrictEquals | ParsedBinaryOperator::StrictNotEquals
                );
                // tsc's `narrowTypeByConstructor`: `x.constructor === C` keeps
                // what `C` constructs on its equal edge, which is never
                // `undefined` — unless `C` is `Object` or `Function`, whose
                // prototypes narrow nothing.
                if let Some((name, constructor)) = constructor_comparison(left, right) {
                    let narrows = !matches!(
                        constructor,
                        ParsedExpression::Identifier { name: constructor, .. }
                            if constructor == "Object" || constructor == "Function"
                    );
                    if equal_edge && narrows {
                        out.push(name);
                    }
                    return;
                }
                if let Some((name, tag)) = typeof_comparison(left, right) {
                    // `typeof x === "undefined"` keeps only `undefined`; any
                    // other tag excludes it.
                    if (tag != "undefined") == equal_edge {
                        out.push(name);
                    }
                    return;
                }
                if let (
                    ParsedExpression::Identifier { name: left_name, .. },
                    ParsedExpression::Identifier { name: right_name, .. },
                ) = (left.as_ref(), right.as_ref())
                {
                    if equal_edge {
                        if (guards.never_undefined)(right_name, !strict) {
                            out.push(left_name);
                        }
                        if (guards.never_undefined)(left_name, !strict) {
                            out.push(right_name);
                        }
                    }
                    return;
                }
                let Some((name, other)) = identifier_comparison(left, right) else {
                    return;
                };
                match other {
                    ParsedExpression::UndefinedLiteral => {
                        if !equal_edge {
                            out.push(name);
                        }
                    }
                    ParsedExpression::Unary {
                        operator: ParsedUnaryOperator::Void,
                        ..
                    } => {
                        if !equal_edge {
                            out.push(name);
                        }
                    }
                    // `x === null` holds only for `null`; `x == null` holds for
                    // `undefined` too, so only its other edge excludes it.
                    ParsedExpression::NullLiteral => {
                        if equal_edge == strict {
                            out.push(name);
                        }
                    }
                    other if is_never_undefined(other) => {
                        if equal_edge {
                            out.push(name);
                        }
                    }
                    _ => {}
                }
            }
            ParsedBinaryOperator::Instanceof => {
                if when && let ParsedExpression::Identifier { name, .. } = left.as_ref() {
                    out.push(name);
                }
            }
            ParsedBinaryOperator::In => {
                if when && let ParsedExpression::Identifier { name, .. } = right.as_ref() {
                    out.push(name);
                }
            }
            _ => {}
        },
        // A `param is T` predicate narrows its argument to `T` when true.
        ParsedExpression::Call {
            callee_name,
            arguments,
            ..
        } => {
            if when
                && let Some(index) = (guards.predicate_parameter)(callee_name)
                && let Some(ParsedExpression::Identifier { name, .. }) =
                    arguments.get(index).map(|argument| &argument.expression)
            {
                out.push(name);
            }
        }
        _ => {}
    }
}

/// `typeof x === "tag"` in either operand order.
fn typeof_comparison<'e>(
    left: &'e ParsedExpression,
    right: &'e ParsedExpression,
) -> Option<(&'e str, &'e str)> {
    let typeof_operand = |expression: &'e ParsedExpression| match expression {
        ParsedExpression::Unary {
            operator: ParsedUnaryOperator::Typeof,
            operand,
            ..
        } => match operand.as_ref() {
            ParsedExpression::Identifier { name, .. } => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    };
    let tag = |expression: &'e ParsedExpression| match expression {
        ParsedExpression::StringLiteral(tag) => Some(tag.as_str()),
        ParsedExpression::TemplateLiteral {
            expressions,
            quasis,
            ..
        } if expressions.is_empty() => quasis.first()?.as_deref(),
        _ => None,
    };
    typeof_operand(left)
        .zip(tag(right))
        .or_else(|| typeof_operand(right).zip(tag(left)))
}

/// `x.constructor` compared with a constructor reference, in either operand
/// order.
fn constructor_comparison<'e>(
    left: &'e ParsedExpression,
    right: &'e ParsedExpression,
) -> Option<(&'e str, &'e ParsedExpression)> {
    let constructor_of = |expression: &'e ParsedExpression| match expression {
        ParsedExpression::PropertyAccess {
            object,
            property_name,
            ..
        } if property_name == "constructor" => match object.as_ref() {
            ParsedExpression::Identifier { name, .. } => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    };
    let is_reference = |expression: &ParsedExpression| {
        matches!(
            expression,
            ParsedExpression::Identifier { .. } | ParsedExpression::PropertyAccess { .. }
        )
    };
    match (constructor_of(left), constructor_of(right)) {
        (Some(name), _) if is_reference(right) => Some((name, right)),
        (_, Some(name)) if is_reference(left) => Some((name, left)),
        _ => None,
    }
}

/// A local compared with some other expression, in either operand order.
fn identifier_comparison<'e>(
    left: &'e ParsedExpression,
    right: &'e ParsedExpression,
) -> Option<(&'e str, &'e ParsedExpression)> {
    match (left, right) {
        (ParsedExpression::Identifier { name, .. }, other)
            if !matches!(other, ParsedExpression::Identifier { .. }) =>
        {
            Some((name, other))
        }
        (other, ParsedExpression::Identifier { name, .. })
            if !matches!(other, ParsedExpression::Identifier { .. }) =>
        {
            Some((name, other))
        }
        _ => None,
    }
}

/// An expression whose type cannot contain `undefined` whatever it refers to.
fn is_never_undefined(expression: &ParsedExpression) -> bool {
    matches!(
        expression,
        ParsedExpression::StringLiteral(_)
            | ParsedExpression::NumberLiteral(_)
            | ParsedExpression::BigIntLiteral(_)
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::ObjectLiteral { .. }
            | ParsedExpression::ArrayLiteral { .. }
    )
}
