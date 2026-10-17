//! Unary, binary, logical, and conditional expression inference.

use super::*;

use surge_ts_syntax::{ParsedBinaryOperator, ParsedExpression, ParsedUnaryOperator};
use surge_ts_types::{Type, union_type};

use crate::context::CheckerContext;
use crate::symbols::SymbolTable;

use crate::infer::InferredExpression;

/// tsc's `!` result (`checkPrefixUnaryExpression`): `false` for an operand
/// that is always truthy, `true` for one always falsy, `boolean` otherwise.
pub(crate) fn logical_not_result_type(operand: &Type) -> Type {
    if matches!(operand.peeled(), Type::Never) {
        return Type::Boolean;
    }
    match crate::checks::function::type_truthiness(operand) {
        Some(truthy) => Type::BooleanLiteral(!truthy),
        None => Type::Boolean,
    }
}

/// tsc's `typeofType`: the union of every `typeof` result, in sorted order.
pub(crate) fn typeof_result_type() -> Type {
    union_type(
        [
            "bigint",
            "boolean",
            "function",
            "number",
            "object",
            "string",
            "symbol",
            "undefined",
        ]
        .into_iter()
        .map(|name| Type::StringLiteral(name.to_string()))
        .collect(),
    )
}

pub(crate) fn infer_unary_expression(
    operator: ParsedUnaryOperator,
    operand: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let operand_type = infer_expression(operand, symbols, ctx);

    match operator {
        ParsedUnaryOperator::Not => match operand_type {
            InferredExpression::Known(ty) if !ty.is_unknown() => {
                InferredExpression::Known(logical_not_result_type(&ty))
            }
            _ => InferredExpression::Unknown,
        },
        ParsedUnaryOperator::Typeof => InferredExpression::Known(typeof_result_type()),
        ParsedUnaryOperator::Delete => InferredExpression::Known(Type::Boolean),
        ParsedUnaryOperator::Void => InferredExpression::Known(Type::Undefined),
        // checker.go `checkPrefixUnaryExpression`: `+x` is always `number`;
        // `-x` and `~x` are `getUnaryResultType`, `bigint` for a `bigint`
        // operand and `number | bigint` for one that may be either.
        ParsedUnaryOperator::Plus | ParsedUnaryOperator::Minus | ParsedUnaryOperator::BitwiseNot => match operand_type {
            InferredExpression::Known(Type::Unknown | Type::ErrorType)
            | InferredExpression::UnresolvedIdentifier { .. }
            | InferredExpression::MissingProperty { .. }
            | InferredExpression::Unknown => InferredExpression::Unknown,
            InferredExpression::Known(_) if matches!(operator, ParsedUnaryOperator::Plus) => {
                InferredExpression::Known(Type::Number)
            }
            InferredExpression::Known(ty) => {
                let peeled = ty.peeled();
                let bigint_like = |member: &Type| matches!(member, Type::BigInt);
                let number_like = |member: &Type| matches!(member.base_primitive(), Some(Type::Number));
                let (has_bigint, has_number) = match &peeled {
                    Type::Union(union) => (
                        union.types().iter().any(bigint_like),
                        union.types().iter().any(number_like),
                    ),
                    other => (bigint_like(other), false),
                };
                InferredExpression::Known(match (has_bigint, has_number || matches!(peeled, Type::Any | Type::GenuineUnknown)) {
                    (false, _) => Type::Number,
                    (true, false) => Type::BigInt,
                    (true, true) => surge_ts_types::union_type(vec![Type::Number, Type::BigInt]),
                })
            }
        },
    }
}

pub(crate) fn infer_logical_expression(
    operator: surge_ts_syntax::ParsedLogicalOperator,
    left: &ParsedExpression,
    right: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let left_type = infer_expression(left, symbols, ctx);
    // `(options || {}).color`: the empty fallback reads the left operand's names
    // as `undefined`, exactly as `options ?? {}` does.
    let empty_fallback = match (&operator, &left_type) {
        (surge_ts_syntax::ParsedLogicalOperator::Or, InferredExpression::Known(known)) => {
            crate::checks::expr::empty_object_fallback_type(right, &truthy_part(known))
        }
        _ => None,
    };
    let right_type = match empty_fallback {
        Some(fallback) => InferredExpression::Known(fallback),
        None => infer_expression(right, symbols, ctx),
    };

    crate::checks::ops::evaluate_logical_expression(operator, left_type, right_type)
}

pub(crate) fn infer_conditional_expression(
    condition: &ParsedExpression,
    when_true: &ParsedExpression,
    when_false: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let condition_type = infer_expression(condition, symbols, ctx);
    if !matches!(&condition_type, InferredExpression::Known(ty) if !ty.is_unmodelled()) {
        return InferredExpression::Unknown;
    }

    // Narrow a discriminated union for each branch: `x.kind === "a" ? x.a : x.b`
    // sees `x` as the `"a"` member in `when_true` and its complement in
    // `when_false`.
    let true_symbols =
        crate::checks::function::narrow_condition_symbol_table(condition, symbols, true);
    let true_symbols = crate::checks::function::narrow_predicate_guards_symbol_table(
        condition,
        true_symbols.as_ref().unwrap_or(symbols),
        true,
        ctx,
    )
    .or(true_symbols);
    let false_symbols =
        crate::checks::function::narrow_condition_symbol_table(condition, symbols, false);
    let false_symbols = crate::checks::function::narrow_predicate_guards_symbol_table(
        condition,
        false_symbols.as_ref().unwrap_or(symbols),
        false,
        ctx,
    )
    .or(false_symbols);
    // `typeof xs[0] === 'string' ? xs[0] : …` — an element access has no binding
    // to narrow, so its guarded read is recorded per access the way the
    // statement-level evaluator already does it.
    let true_symbols = crate::checks::function::narrow_element_reference_guards_symbol_table(
        condition,
        true,
        true_symbols.as_ref().unwrap_or(symbols),
        ctx,
    )
    .or(true_symbols);
    let false_symbols = crate::checks::function::narrow_element_reference_guards_symbol_table(
        condition,
        false,
        false_symbols.as_ref().unwrap_or(symbols),
        ctx,
    )
    .or(false_symbols);
    let true_type = infer_expression(when_true, true_symbols.as_ref().unwrap_or(symbols), ctx);
    let false_type = infer_expression(when_false, false_symbols.as_ref().unwrap_or(symbols), ctx);

    match (true_type, false_type) {
        (InferredExpression::Known(Type::Any), _) | (_, InferredExpression::Known(Type::Any)) => {
            InferredExpression::Known(Type::Any)
        }
        (InferredExpression::Known(true_ty), InferredExpression::Known(false_ty))
            if (!true_ty.is_unknown() || true_ty.is_type_variable())
                && (!false_ty.is_unknown() || false_ty.is_type_variable()) =>
        {
            if true_ty == false_ty {
                InferredExpression::Known(true_ty)
            } else {
                InferredExpression::Known(surge_ts_types::subtype_reduced_union(vec![
                    (true_ty, crate::checks::expr::literal_shape(when_true)),
                    (false_ty, crate::checks::expr::literal_shape(when_false)),
                ]))
            }
        }
        _ => InferredExpression::Unknown,
    }
}

pub(crate) fn infer_binary_expression(
    operator: ParsedBinaryOperator,
    left: &ParsedExpression,
    right: &ParsedExpression,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    match operator {
        ParsedBinaryOperator::StrictEquals
        | ParsedBinaryOperator::StrictNotEquals
        | ParsedBinaryOperator::Equals
        | ParsedBinaryOperator::NotEquals
        | ParsedBinaryOperator::LessThan
        | ParsedBinaryOperator::LessThanEquals
        | ParsedBinaryOperator::GreaterThan
        | ParsedBinaryOperator::GreaterThanEquals
        | ParsedBinaryOperator::In
        | ParsedBinaryOperator::Instanceof => InferredExpression::Known(Type::Boolean),
        ParsedBinaryOperator::Add => {
            let left_type = infer_expression(left, symbols, ctx);
            let right_type = infer_expression(right, symbols, ctx);

            match (left_type, right_type) {
                // One string operand makes `+` a concatenation whatever the
                // other side is, which is what keeps `'data' + String(x)` a
                // string when the other operand is one this pass cannot type.
                // Falling through to the numeric default instead disagreed with
                // the checking pass, and a generic call inferring from such an
                // arrow bound its return to `number`.
                (InferredExpression::Known(left_ty), _)
                    if matches!(left_ty.base_primitive(), Some(Type::String)) =>
                {
                    InferredExpression::Known(Type::String)
                }
                (_, InferredExpression::Known(right_ty))
                    if matches!(right_ty.base_primitive(), Some(Type::String)) =>
                {
                    InferredExpression::Known(Type::String)
                }
                (InferredExpression::Known(Type::Any), _)
                | (_, InferredExpression::Known(Type::Any)) => InferredExpression::Known(Type::Any),
                (InferredExpression::Known(left_ty), InferredExpression::Known(right_ty))
                    if matches!(left_ty.base_primitive(), Some(Type::Number))
                        && matches!(right_ty.base_primitive(), Some(Type::Number)) =>
                {
                    InferredExpression::Known(Type::Number)
                }
                _ => InferredExpression::Known(Type::Number),
            }
        }
        ParsedBinaryOperator::Subtract
        | ParsedBinaryOperator::Multiply
        | ParsedBinaryOperator::Divide
        | ParsedBinaryOperator::Remainder
        | ParsedBinaryOperator::Exponential
        | ParsedBinaryOperator::ShiftLeft
        | ParsedBinaryOperator::ShiftRight
        | ParsedBinaryOperator::ShiftRightZeroFill
        | ParsedBinaryOperator::BitwiseAnd
        | ParsedBinaryOperator::BitwiseOR
        | ParsedBinaryOperator::BitwiseXOR => InferredExpression::Known(Type::Number),
    }
}

/// The part of `ty` that is falsy, per tsc's `TypeFacts.Falsy`: a nullish or
/// `false`-able member survives, a primitive contributes its falsy literal, and
/// an object never is. `any` and the sentinels are left whole.
pub(crate) fn falsy_part(ty: &Type) -> Type {
    match ty {
        Type::Any | Type::Unknown | Type::GenuineUnknown | Type::TypeParameter(_) => ty.clone(),
        Type::Undefined | Type::Void => Type::Undefined,
        Type::Null => Type::Null,
        // `0n`, which surge has no literal type for.
        Type::BigInt => Type::BigInt,
        Type::Boolean => Type::BooleanLiteral(false),
        Type::BooleanLiteral(false) => ty.clone(),
        Type::String => Type::StringLiteral(String::new()),
        Type::StringLiteral(value) if value.is_empty() => ty.clone(),
        Type::Number => Type::NumberLiteral(surge_ts_types::NumberLiteralType {
            value: "0".to_string(),
        }),
        Type::NumberLiteral(literal) if literal.value == "0" => ty.clone(),
        Type::Union(union) => union_type(union.types().iter().map(falsy_part).collect()),
        Type::Reference(reference) => falsy_part(&reference.resolve()),
        _ => Type::Never,
    }
}

/// What is left of `ty` once it is known truthy — the left operand of `a || b`
/// when it does not fall through. tsc's `removeDefinitelyFalsyTypes` under
/// `getNonNullableType`: the nullish members and the falsy literals go, and
/// `boolean` is `true`. A primitive keeps its type, since only one of its
/// values is falsy, and a named reference is opened only when that changes it.
pub(crate) fn truthy_part(ty: &Type) -> Type {
    match ty {
        Type::Undefined | Type::Void | Type::Null | Type::BooleanLiteral(false) => Type::Never,
        Type::StringLiteral(value) if value.is_empty() => Type::Never,
        Type::NumberLiteral(literal) if literal.value == "0" => Type::Never,
        Type::Boolean => Type::BooleanLiteral(true),
        Type::Union(union) => {
            let parts: Vec<Type> = union.types().iter().map(truthy_part).collect();
            if parts.iter().zip(union.types()).all(|(part, member)| matches!(part, Type::Never) || part == member) {
                surge_ts_types::filter_union(union, |member| !matches!(truthy_part(member), Type::Never))
            } else {
                union_type(parts)
            }
        }
        Type::TypeParameter(_) => surge_ts_types::type_variable::non_nullable_type_variable(ty),
        Type::Reference(reference) if reference.enum_owner.is_none() => match reference.resolve() {
            resolved @ (Type::Union(_) | Type::Boolean) => truthy_part(&resolved),
            _ => ty.clone(),
        },
        _ => ty.clone(),
    }
}
