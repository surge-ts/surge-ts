use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedBinaryOperator, ParsedUnaryOperator, TextSpan as SyntaxTextSpan};
use surge_ts_types::{Type, union_type};

use crate::checks::expr::{operand_display_name, widen_type};
use crate::context::{CheckerContext, convert_span};
use crate::infer::InferredExpression;

pub(crate) fn evaluate_binary_expression(
    left_result: InferredExpression,
    right_result: InferredExpression,
    operator: ParsedBinaryOperator,
    left_span: Option<SyntaxTextSpan>,
    operator_span: Option<SyntaxTextSpan>,
    right_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    compound: bool,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    match operator {
        ParsedBinaryOperator::Add => evaluate_add_binary(
            left_result,
            right_result,
            // The lowering of `x += v` to `x = x + v` writes no operator span.
            operator_span.is_none(),
            left_span.or(fallback_span),
            operator_span.or(fallback_span),
            right_span.or(fallback_span),
            ctx,
        ),
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
        | ParsedBinaryOperator::BitwiseXOR => evaluate_arithmetic_binary(
            left_result,
            right_result,
            operator,
            left_span.or(fallback_span),
            operator_span,
            right_span.or(fallback_span),
            fallback_span,
            compound,
            ctx,
        ),
        ParsedBinaryOperator::LessThan
        | ParsedBinaryOperator::LessThanEquals
        | ParsedBinaryOperator::GreaterThan
        | ParsedBinaryOperator::GreaterThanEquals => evaluate_comparison_binary(
            left_result,
            right_result,
            binary_operator_text(operator),
            left_span.or(fallback_span),
            right_span.or(fallback_span),
            operator_span.or(fallback_span),
            ctx,
        ),
        ParsedBinaryOperator::StrictEquals
        | ParsedBinaryOperator::StrictNotEquals
        | ParsedBinaryOperator::Equals
        | ParsedBinaryOperator::NotEquals => evaluate_equality_binary(
            left_result,
            right_result,
            operator_span.or(fallback_span),
            ctx,
        ),
        // `"prop" in obj` / `x instanceof Ctor` are boolean type-guard tests. The
        // operands are already evaluated by the caller; the result is `boolean`.
        ParsedBinaryOperator::In => InferredExpression::Known(Type::Boolean),
        ParsedBinaryOperator::Instanceof => {
            crate::checks::expr::check_instanceof_left_operand(
                &left_result,
                left_span.or(fallback_span),
                ctx,
            );
            crate::checks::expr::check_instanceof_right_operand(
                &right_result,
                right_span.or(fallback_span),
                ctx,
            );
            InferredExpression::Known(Type::Boolean)
        }
    }
}

fn is_truthy_literal(ty: &Type) -> bool {
    match ty {
        Type::StringLiteral(value) => !value.is_empty(),
        Type::NumberLiteral(literal) => literal.value != "0",
        Type::BooleanLiteral(value) => *value,
        Type::Union(union) => union.types().iter().all(is_truthy_literal),
        _ => false,
    }
}

pub(crate) fn evaluate_logical_expression(
    operator: surge_ts_syntax::ParsedLogicalOperator,
    left_result: InferredExpression,
    right_result: InferredExpression,
) -> InferredExpression {
    let (InferredExpression::Known(left_ty), InferredExpression::Known(right_ty)) =
        (&left_result, &right_result)
    else {
        return InferredExpression::Unknown;
    };
    if left_ty.is_unmodelled() || right_ty.is_unmodelled() {
        return InferredExpression::Unknown;
    }

    // A logical expression yields one of its operand *values*, not `boolean`:
    // `a || b` is `truthy(a) | b` (the left's falsy members are gone when it does
    // not fall through), and `a && b` is `falsy(a) | b` (`a`'s falsy part when it
    // stops the chain, otherwise `b`). `??` has its own handler. Modelling the
    // operand union avoids false assignability errors like
    // `string | undefined || "x"` being treated as `boolean`.
    let result = match operator {
        // A left that is never falsy is the whole result.
        surge_ts_syntax::ParsedLogicalOperator::Or => {
            // Without `strictNullChecks` every object, function and symbol type
            // carries `TypeFacts.Falsy`; only a truthy literal never falls through.
            let never_falsy = if surge_ts_types::strict_null_checks() {
                matches!(crate::infer::falsy_part(left_ty), Type::Never)
            } else {
                is_truthy_literal(left_ty)
            };
            if never_falsy {
                left_ty.clone()
            } else {
                surge_ts_types::union_type(vec![crate::infer::truthy_part(left_ty), right_ty.clone()])
            }
        }
        // tsc's `checkBinaryLikeExpression`: the falsy part comes from the
        // left under `strictNullChecks` and from the right's base type
        // without it, and a left that is never truthy is the whole result.
        surge_ts_syntax::ParsedLogicalOperator::And => {
            if matches!(crate::infer::truthy_part(left_ty), Type::Never) {
                left_ty.clone()
            } else {
                let falsy_source = if surge_ts_types::strict_null_checks() {
                    left_ty.clone()
                } else {
                    crate::checks::expr::widen_type(right_ty)
                };
                surge_ts_types::union_type(vec![crate::infer::falsy_part(&falsy_source), right_ty.clone()])
            }
        }
    };
    InferredExpression::Known(result)
}

pub(crate) fn evaluate_conditional_expression(
    condition_result: InferredExpression,
    true_result: InferredExpression,
    false_result: InferredExpression,
) -> InferredExpression {
    if !matches!(&condition_result, InferredExpression::Known(ty) if !ty.is_unmodelled()) {
        return InferredExpression::Unknown;
    }

    let Some(true_type) = inferred_type(&true_result) else {
        return InferredExpression::Unknown;
    };
    let Some(false_type) = inferred_type(&false_result) else {
        return InferredExpression::Unknown;
    };

    if (true_type.is_unmodelled() && !true_type.is_type_variable())
        || (false_type.is_unmodelled() && !false_type.is_type_variable())
    {
        return InferredExpression::Unknown;
    }

    if matches!(true_type, Type::Any) || matches!(false_type, Type::Any) {
        return InferredExpression::Known(Type::Any);
    }

    if true_type == false_type {
        return InferredExpression::Known(true_type.clone());
    }

    InferredExpression::Known(union_type(vec![true_type.clone(), false_type.clone()]))
}

pub(crate) fn evaluate_unary_expression(
    operator: ParsedUnaryOperator,
    operand_result: InferredExpression,
) -> InferredExpression {
    match operator {
        ParsedUnaryOperator::Not => match inferred_type(&operand_result) {
            Some(operand_type) if !operand_type.is_unknown() => InferredExpression::Known(
                crate::infer::expression::logical_not_result_type(&operand_type),
            ),
            _ => InferredExpression::Unknown,
        },
        ParsedUnaryOperator::Typeof => {
            InferredExpression::Known(crate::infer::expression::typeof_result_type())
        }
        ParsedUnaryOperator::Delete => InferredExpression::Known(Type::Boolean),
        ParsedUnaryOperator::Void => InferredExpression::Known(Type::Undefined),
        // Unary `+`/`-` coerce: tsc accepts any operand and types the result
        // `number` (`bigint` for a bigint operand). TS2356 is the `++`/`--`
        // operand rule, not this one — reporting it here made `+data` on a
        // contextually-typed `string` parameter a false positive.
        ParsedUnaryOperator::Plus | ParsedUnaryOperator::Minus | ParsedUnaryOperator::BitwiseNot => {
            let Some(operand_type) = inferred_type(&operand_result) else {
                return InferredExpression::Unknown;
            };

            if operand_type.is_unknown() {
                return InferredExpression::Unknown;
            }

            // Unary `+` is always `number`, `any` operand included; it rejects
            // a bigint operand instead. `-`/`~` are `getUnaryResultType`.
            if !matches!(operator, ParsedUnaryOperator::Plus) {
                let peeled = operand_type.peeled();
                if matches!(peeled.base_primitive(), Some(Type::BigInt)) {
                    return InferredExpression::Known(Type::BigInt);
                }
                if let Type::Union(union) = &peeled
                    && union.types().iter().any(|member| matches!(member, Type::BigInt))
                {
                    let number_like = union
                        .types()
                        .iter()
                        .any(|member| matches!(member.base_primitive(), Some(Type::Number)));
                    return InferredExpression::Known(if number_like {
                        surge_ts_types::union_type(vec![Type::Number, Type::BigInt])
                    } else {
                        Type::BigInt
                    });
                }
            }

            InferredExpression::Known(Type::Number)
        }
    }
}

fn evaluate_add_binary(
    left_result: InferredExpression,
    right_result: InferredExpression,
    compound_assignment: bool,
    left_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    right_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let operand_type = |result: &InferredExpression| match result {
        InferredExpression::MissingProperty { .. } => Some(Type::ErrorType),
        other => inferred_type(other).cloned(),
    };
    let Some(left_type) = operand_type(&left_result) else {
        return InferredExpression::Unknown;
    };
    let Some(right_type) = operand_type(&right_result) else {
        return InferredExpression::Unknown;
    };

    // The operands may be nominal references (`core.output<T>` around the
    // awaited/inferred type). Every classification below matches on structure,
    // so a reference that resolves to `any`/`string`/`number` has to be peeled
    // first or it falls through to the report.
    let left_type = left_type.peeled();
    let right_type = right_type.peeled();

    // tsc's error type is an `any`: beside a string operand the result is
    // still `string`.
    let string_beside_error = |error: &Type, other: &Type| {
        matches!(error, Type::ErrorType) && is_strictly_assignable_to(other, &Type::String)
    };
    if string_beside_error(&left_type, &right_type) || string_beside_error(&right_type, &left_type) {
        return InferredExpression::Known(Type::String);
    }
    if is_unmodelled(&left_type) || is_unmodelled(&right_type) {
        return InferredExpression::Unknown;
    }

    // tsc's `+` arm of `checkBinaryLikeExpression`: the result kind is decided
    // first, and only an operand pair with no result kind is an error. A
    // symbol cannot join even a well-typed `+` (`checkForDisallowedESSymbolOperand`).
    let both = |target: &Type| {
        is_strictly_assignable_to(&left_type, target) && is_strictly_assignable_to(&right_type, target)
    };
    let result = if both(&Type::Number) {
        Some(Type::Number)
    } else if both(&Type::BigInt) {
        Some(Type::BigInt)
    } else if is_strictly_assignable_to(&left_type, &Type::String)
        || is_strictly_assignable_to(&right_type, &Type::String)
    {
        Some(Type::String)
    } else if matches!(left_type, Type::Any) || matches!(right_type, Type::Any) {
        Some(Type::Any)
    } else {
        None
    };
    let operator_text = if compound_assignment { "+=" } else { "+" };
    if let Some(result) = result {
        // `+=` checks its assignment only when no symbol operand was reported.
        if report_symbol_operand(operator_text, &left_type, &right_type, left_span, right_span, ctx)
            && compound_assignment
        {
            return InferredExpression::Unknown;
        }
        return InferredExpression::Known(result);
    }

    // `getBaseTypesIfUnrelated`: the operands keep their literal names when
    // their base types are both ones `+` could plausibly have meant.
    let close_enough = |ty: &Type| {
        *ty == Type::GenuineUnknown
            || [Type::Number, Type::BigInt, Type::String]
            .iter()
            .any(|target| surge_ts_types::is_assignable_to(ty, target))
    };
    let (left_base, right_base) = (widen_type(&left_type), widen_type(&right_type));
    let (left_name, right_name) = if close_enough(&left_base) && close_enough(&right_base) {
        (left_type.name(), right_type.name())
    } else {
        (left_base.name(), right_base.name())
    };
    let file_name = ctx.file_name.clone();
    push_diagnostic(
        ctx,
        Diagnostic::ts2365(operator_text, &left_name, &right_name, file_name),
        fallback_span,
    );
    InferredExpression::Unknown
}

/// `isTypeAssignableToKindEx(ty, kind, strict = true)`: the operand relates to
/// the kind's primitive, where `any`, `unknown`, `void`, `undefined` and `null`
/// do not count on their own.
fn is_strictly_assignable_to(ty: &Type, target: &Type) -> bool {
    !matches!(ty, Type::Any | Type::Void | Type::Undefined | Type::Null)
        && !ty.is_unmodelled()
        && surge_ts_types::is_assignable_to(ty, target)
}

/// TS2469 on the first operand that may be a symbol. Returns whether one was.
fn report_symbol_operand(
    operator_text: &str,
    left_type: &Type,
    right_type: &Type,
    left_span: Option<SyntaxTextSpan>,
    right_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> bool {
    fn maybe_symbol(ty: &Type) -> bool {
        match ty {
            Type::Symbol => true,
            Type::Union(union) => union.types().iter().any(maybe_symbol),
            _ => false,
        }
    }
    let span = if maybe_symbol(left_type) {
        left_span
    } else if maybe_symbol(right_type) {
        right_span
    } else {
        return false;
    };
    let file_name = ctx.file_name.clone();
    push_diagnostic(ctx, Diagnostic::ts2469(operator_text, file_name), span);
    true
}

/// tsc's arithmetic and bitwise arm of `checkBinaryLikeExpression`: two
/// boolean operands of `&`/`|`/`^` are TS2447 on the operator; otherwise each
/// operand must be `any`, number-like or bigint-like (TS2362/TS2363), and
/// mixing a bigint with a non-bigint — or `>>>` on bigints — is TS2365 on the
/// whole expression.
#[allow(clippy::too_many_arguments)]
fn evaluate_arithmetic_binary(
    left_result: InferredExpression,
    right_result: InferredExpression,
    operator: ParsedBinaryOperator,
    left_span: Option<SyntaxTextSpan>,
    operator_span: Option<SyntaxTextSpan>,
    right_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    compound: bool,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    // `errorType` is `any` to `checkArithmeticOperandType`: the other
    // operand is still checked.
    let is_error = |result: &InferredExpression| {
        matches!(
            result,
            InferredExpression::UnresolvedIdentifier { .. }
                | InferredExpression::MissingProperty { .. }
                | InferredExpression::Known(Type::ErrorType)
        )
    };
    if is_error(&left_result) || is_error(&right_result) {
        let invalid = |result: &InferredExpression| {
            !is_error(result)
                && inferred_type(result).is_some_and(|ty| !is_unmodelled(ty) && !is_valid_arithmetic_operand(ty))
        };
        if invalid(&left_result) {
            let file_name = ctx.file_name.clone();
            push_diagnostic(ctx, Diagnostic::ts2362(file_name), left_span);
        }
        if invalid(&right_result) {
            let file_name = ctx.file_name.clone();
            push_diagnostic(ctx, Diagnostic::ts2363(file_name), right_span);
        }
        return InferredExpression::Unknown;
    }
    let Some(left_type) = inferred_type(&left_result) else {
        return InferredExpression::Unknown;
    };
    let Some(right_type) = inferred_type(&right_result) else {
        return InferredExpression::Unknown;
    };

    if is_unmodelled(&left_type) || is_unmodelled(&right_type) {
        return InferredExpression::Unknown;
    }

    if let Some(suggested) = suggested_boolean_operator(operator)
        && is_boolean_like(left_type)
        && is_boolean_like(right_type)
    {
        let file_name = ctx.file_name.clone();
        push_diagnostic(
            ctx,
            Diagnostic::ts2447(binary_operator_text(operator), suggested, file_name),
            operator_span.or(fallback_span),
        );
        return InferredExpression::Known(Type::Number);
    }

    let left_valid = is_valid_arithmetic_operand(left_type);
    let right_valid = is_valid_arithmetic_operand(right_type);
    if !left_valid {
        let file_name = ctx.file_name.clone();
        push_diagnostic(ctx, Diagnostic::ts2362(file_name), left_span);
    }
    if !right_valid {
        let file_name = ctx.file_name.clone();
        push_diagnostic(ctx, Diagnostic::ts2363(file_name), right_span);
    }

    // Without a bigint in play the result is `number`, `any` operands included.
    // A compound assignment with an invalid operand is not checked as an
    // assignment (`checkAssignmentOperator` runs only when both are valid).
    if !maybe_bigint_like(left_type) && !maybe_bigint_like(right_type) {
        if compound && !(left_valid && right_valid) {
            return InferredExpression::Unknown;
        }
        return InferredExpression::Known(Type::Number);
    }
    if is_bigint_like(left_type) && is_bigint_like(right_type) {
        if matches!(operator, ParsedBinaryOperator::ShiftRightZeroFill) {
            report_operator_error(operator, left_type, right_type, fallback_span, ctx);
        }
        if matches!(operator, ParsedBinaryOperator::Exponential) && !ctx.options.target_es2016 {
            let file_name = ctx.file_name.clone();
            push_diagnostic(ctx, Diagnostic::ts2791(file_name), fallback_span);
        }
        return InferredExpression::Known(Type::BigInt);
    }
    report_operator_error(operator, left_type, right_type, fallback_span, ctx);
    InferredExpression::Unknown
}

fn report_operator_error(
    operator: ParsedBinaryOperator,
    left_type: &Type,
    right_type: &Type,
    span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let file_name = ctx.file_name.clone();
    push_diagnostic(
        ctx,
        Diagnostic::ts2365(
            binary_operator_text(operator),
            &operand_display_name(left_type),
            &operand_display_name(right_type),
            file_name,
        ),
        span,
    );
}

fn suggested_boolean_operator(operator: ParsedBinaryOperator) -> Option<&'static str> {
    match operator {
        ParsedBinaryOperator::BitwiseAnd => Some("&&"),
        ParsedBinaryOperator::BitwiseOR => Some("||"),
        ParsedBinaryOperator::BitwiseXOR => Some("!=="),
        _ => None,
    }
}

fn is_boolean_like(ty: &Type) -> bool {
    match ty {
        Type::Boolean | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(is_boolean_like),
        _ => false,
    }
}

/// `isTypeAssignableToKind(t, BigIntLike)`, which `any` satisfies.
fn is_bigint_like(ty: &Type) -> bool {
    match ty {
        Type::BigInt | Type::Any => true,
        Type::Union(union) => union.types().iter().all(is_bigint_like),
        Type::TypeParameter(_) => surge_ts_types::is_assignable_to(ty, &Type::BigInt),
        _ => false,
    }
}

fn maybe_bigint_like(ty: &Type) -> bool {
    match ty {
        Type::BigInt => true,
        Type::Union(union) => union.types().iter().any(maybe_bigint_like),
        Type::TypeParameter(_) => surge_ts_types::is_assignable_to(ty, &Type::BigInt),
        _ => false,
    }
}

/// `checkArithmeticOperandType`: `any`, number-like (numeric enums included)
/// or bigint-like — `isTypeAssignableTo(t, numberOrBigIntType)`, which a type
/// variable meets through its constraint.
fn is_valid_arithmetic_operand(ty: &Type) -> bool {
    match ty {
        // `never` is assignable to `number | bigint` like to anything.
        Type::Any | Type::BigInt | Type::Never => true,
        Type::Union(union) => union.types().iter().all(is_valid_arithmetic_operand),
        Type::TypeParameter(_) => {
            surge_ts_types::is_assignable_to(ty, &union_type(vec![Type::Number, Type::BigInt]))
        }
        other => is_number_like_for_arithmetic(other),
    }
}

fn evaluate_comparison_binary(
    left_result: InferredExpression,
    right_result: InferredExpression,
    operator_text: &'static str,
    left_span: Option<SyntaxTextSpan>,
    right_span: Option<SyntaxTextSpan>,
    fallback_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let Some(left_type) = inferred_type(&left_result) else {
        return InferredExpression::Unknown;
    };
    let Some(right_type) = inferred_type(&right_result) else {
        return InferredExpression::Unknown;
    };

    // A type variable of the body being checked is a real operand, related
    // through its constraint.
    if is_unmodelled(left_type) || is_unmodelled(right_type) {
        return InferredExpression::Unknown;
    }

    if report_symbol_operand(operator_text, left_type, right_type, left_span, right_span, ctx) {
        return InferredExpression::Known(Type::Boolean);
    }

    if matches!(left_type, Type::Any) || matches!(right_type, Type::Any) {
        return InferredExpression::Known(Type::Boolean);
    }

    // tsc's `checkBinaryLikeExpression` for `<`/`>`/`<=`/`>=`: literals are
    // compared by their base type, and then both operands must be numeric
    // (`number | bigint`), or neither numeric and comparable to each other in
    // either direction (`areTypesComparable`).
    let left_base = widen_literal_for_comparison(left_type);
    let right_base = widen_literal_for_comparison(right_type);
    let left_numeric = is_numeric_comparison_operand(&left_base);
    let right_numeric = is_numeric_comparison_operand(&right_base);
    let comparable = || {
        surge_ts_types::is_comparable_to(&left_base, &right_base)
            || surge_ts_types::is_comparable_to(&right_base, &left_base)
    };
    if (left_numeric && right_numeric) || (!left_numeric && !right_numeric && comparable()) {
        return InferredExpression::Known(Type::Boolean);
    }
    let file_name = ctx.file_name.clone();
    push_diagnostic(
        ctx,
        Diagnostic::ts2365(
            operator_text,
            &operand_display_name(&left_base),
            &operand_display_name(&right_base),
            file_name,
        ),
        fallback_span,
    );
    InferredExpression::Unknown
}

fn evaluate_equality_binary(
    left_result: InferredExpression,
    right_result: InferredExpression,
    fallback_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let Some(left_type) = inferred_type(&left_result) else {
        return InferredExpression::Unknown;
    };
    let Some(right_type) = inferred_type(&right_result) else {
        return InferredExpression::Unknown;
    };

    // A type variable of the body being checked is a real operand: two of
    // them are comparable only when one is constrained to the other.
    if left_type.is_unmodelled() || right_type.is_unmodelled() {
        return InferredExpression::Unknown;
    }

    if matches!(left_type, Type::Any) || matches!(right_type, Type::Any) {
        return InferredExpression::Known(Type::Boolean);
    }

    if !types_overlap_for_equality(left_type, right_type) {
        let file_name = ctx.file_name.clone();
        let (left_name, right_name) = equality_operand_display_names(left_type, right_type);
        push_diagnostic(
            ctx,
            Diagnostic::ts2367(&left_name, &right_name, file_name),
            fallback_span,
        );
    }

    InferredExpression::Known(Type::Boolean)
}

/// Operand names for TS2367, mirroring tsc's `getBaseTypesIfUnrelated`: the
/// widened operands are reported only when widening does not make them
/// comparable. `1 === "string"` reads `'number'` and `'string'`, while
/// `"a" === "b"` and a literal-union subject keep their literal names.
pub(crate) fn equality_operand_display_names(left: &Type, right: &Type) -> (String, String) {
    let left_base = widen_type(left);
    let right_base = widen_type(right);
    if types_overlap_for_equality(&left_base, &right_base) {
        return (left.name(), right.name());
    }
    (left_base.name(), right_base.name())
}

pub(crate) fn inferred_type(result: &InferredExpression) -> Option<&Type> {
    match result {
        InferredExpression::Known(ty) => Some(ty),
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. }
        | InferredExpression::Unknown => None,
    }
}

fn is_number_like_for_arithmetic(ty: &Type) -> bool {
    matches!(ty.base_primitive(), Some(Type::Number))
}

/// `isTypeAssignableTo(t, numberOrBigIntType)`, which `never` satisfies.
fn is_numeric_comparison_operand(ty: &Type) -> bool {
    surge_ts_types::is_assignable_to(ty, &union_type(vec![Type::Number, Type::BigInt]))
}

/// `getBaseTypeOfLiteralTypeForComparison`: enum members and enums read as
/// the primitive their values are, not as the enum.
fn widen_literal_for_comparison(ty: &Type) -> Type {
    match ty {
        Type::StringLiteral(_) => Type::String,
        Type::NumberLiteral(_) => Type::Number,
        Type::BooleanLiteral(_) => Type::Boolean,
        Type::Union(union) => {
            surge_ts_types::union_type(union.types().iter().map(widen_literal_for_comparison).collect())
        }
        Type::Reference(_)
            if surge_ts_types::is_template_literal_type(ty)
                || surge_ts_types::string_mapping_parts(ty).is_some() =>
        {
            Type::String
        }
        Type::Reference(reference) if reference.enum_owner.is_some() => {
            widen_literal_for_comparison(&ty.peeled())
        }
        other => other.clone(),
    }
}

/// tsc's equality check (`checkBinaryLikeExpression`): the operands overlap
/// when either is `isTypeEqualityComparableTo` the other — the other operand
/// is `undefined` or `null` as a whole, or the comparable relation holds.
pub(crate) fn types_overlap_for_equality(left: &Type, right: &Type) -> bool {
    is_type_equality_comparable_to(left, right) || is_type_equality_comparable_to(right, left)
}

fn is_type_equality_comparable_to(source: &Type, target: &Type) -> bool {
    matches!(target, Type::Undefined | Type::Null) || surge_ts_types::is_comparable_to(source, target)
}

fn push_diagnostic(ctx: &mut CheckerContext, diagnostic: Diagnostic, span: Option<SyntaxTextSpan>) {
    let diagnostic = match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    };

    ctx.push(diagnostic);
}

fn binary_operator_text(operator: ParsedBinaryOperator) -> &'static str {
    match operator {
        ParsedBinaryOperator::LessThan => "<",
        ParsedBinaryOperator::LessThanEquals => "<=",
        ParsedBinaryOperator::GreaterThan => ">",
        ParsedBinaryOperator::GreaterThanEquals => ">=",
        ParsedBinaryOperator::Add => "+",
        ParsedBinaryOperator::Subtract => "-",
        ParsedBinaryOperator::Multiply => "*",
        ParsedBinaryOperator::Divide => "/",
        ParsedBinaryOperator::Remainder => "%",
        ParsedBinaryOperator::Exponential => "**",
        ParsedBinaryOperator::ShiftLeft => "<<",
        ParsedBinaryOperator::ShiftRight => ">>",
        ParsedBinaryOperator::ShiftRightZeroFill => ">>>",
        ParsedBinaryOperator::BitwiseAnd => "&",
        ParsedBinaryOperator::BitwiseOR => "|",
        ParsedBinaryOperator::BitwiseXOR => "^",
        ParsedBinaryOperator::StrictEquals
        | ParsedBinaryOperator::StrictNotEquals
        | ParsedBinaryOperator::Equals
        | ParsedBinaryOperator::NotEquals => "==",
        ParsedBinaryOperator::In => "in",
        ParsedBinaryOperator::Instanceof => "instanceof",
    }
}

/// A type surge could not model, as opposed to the written `unknown`, which an
/// operator judges like any other operand (under `strictNullChecks` it is
/// reported before it gets here), and to a type variable of the body being
/// checked, which relates through its constraint.
fn is_unmodelled(ty: &Type) -> bool {
    ty.is_unmodelled() && *ty != Type::GenuineUnknown
}
