use std::collections::BTreeMap;
use std::time::Instant;
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedExpression, ParsedObjectProperty, TextSpan as SyntaxTextSpan};
use surge_ts_types::{ObjectProperty, Type, is_assignable_to};

use super::expr::{evaluate_expression, source_display_name};
use crate::context::CheckerContext;
use crate::infer::InferredExpression;
use crate::metrics::alloc_object_type;
use crate::program::{
    record_assignability_check, record_object_literal_property_check, record_program_timing,
};
use crate::spans::{choose_span, diagnostic_with_syntax_span};
use crate::symbols::SymbolTable;
use surge_ts_types::{TypeCopyReason, with_type_copy_reason};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpectedTypeDiagnostic {
    TypeNotAssignable,
    ArgumentNotAssignable,
    SatisfiesNotAssignable,
    /// The expectation types the expression — its callbacks' parameters, its
    /// methods' — and relates it to nothing. Go gives the operand of `x as T` and
    /// `<T>x` the asserted type as its contextual type (checker.go:29718) but
    /// never checks assignability against it: an assertion is related by
    /// comparability, which TS2352 reports on its own.
    ContextOnly,
}

fn is_fresh_literal_operand(operand: &ParsedExpression) -> bool {
    matches!(operand, ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. })
}

/// An operand's own type, which its contextual type does not replace; what
/// reading it reports was reported when it was evaluated in context.
fn own_operand_type(
    operand: &ParsedExpression,
    span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let before = ctx.diagnostics().len();
    let own = evaluate_expression(operand, span, symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(before);
    own
}

/// A literal operand's type as tsc's `checkExpression` gives it under the
/// contextual type: context only keeps literals, except that an array literal
/// in a tuple context is the tuple (`first ?? [undefined]`).
fn literal_operand_own_type(
    operand: &ParsedExpression,
    contextual_result: &InferredExpression,
    span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    if matches!(operand, ParsedExpression::ArrayLiteral { .. })
        && matches!(contextual_result, InferredExpression::Known(ty)
            if matches!(ty, Type::OpenTuple(_)) || surge_ts_types::fixed_tuple_parts(ty).is_some())
    {
        return contextual_result.clone();
    }
    own_operand_type(operand, span, symbols, ctx)
}

/// `a || b` and `a ?? b` are related as one value (`elaborateError` enters
/// neither operand), reported where the target is.
fn relate_joined_operand_value(
    joined: &InferredExpression,
    expected_type: &Type,
    span: Option<SyntaxTextSpan>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    ctx: &mut CheckerContext,
) {
    let InferredExpression::Known(value) = joined else {
        return;
    };
    if expected_diagnostic != ExpectedTypeDiagnostic::TypeNotAssignable
        || value.is_unmodelled()
        || crate::checks::function::type_contains_degradation(value)
        || crate::checks::function::type_contains_degradation(expected_type)
        || is_assignable_to(value, expected_type)
    {
        return;
    }
    let target = crate::checks::expr::reported_relation_target(value, expected_type);
    let source_name = crate::checks::expr::source_display_name(value, &target);
    let diagnostic = crate::checks::expr::type_not_assignable_diagnostic(
        value,
        &target,
        &source_name,
        &target.name(),
        ctx.file_name.clone(),
    );
    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
}

pub(crate) fn evaluate_expression_with_expected_type(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    expected_type: Option<&Type>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    evaluate_expression_with_expected_type_anchored(
        expression,
        fallback_span,
        None,
        expected_type,
        expected_diagnostic,
        symbols,
        ctx,
    )
}

/// Like [`evaluate_expression_with_expected_type`], but threads a `target_span`
/// that whole-value assignability diagnostics (TS2741 missing property and the
/// top-level object mismatch) anchor on, matching tsc which points such errors at
/// the assignment target (e.g. the declaration name) rather than the value. When
/// `target_span` is `None` the behavior is identical to the unanchored entry.
pub(crate) fn evaluate_expression_with_expected_type_anchored(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_type: Option<&Type>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let before = ctx.diagnostics().len();
    let result = evaluate_anchored(
        expression,
        fallback_span,
        target_span,
        expected_type,
        expected_diagnostic,
        symbols,
        ctx,
    );
    reanchor_on_error_span(expression, fallback_span, before, ctx);
    crate::checks::expr::note_evaluation_certainty(&result, ctx);
    result
}

/// `getErrorSpanForNode`: a relation reported on a whole function or class
/// expression is shown at its name or first token (an arrow's first line).
pub(crate) fn reanchor_on_error_span(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    before: usize,
    ctx: &mut CheckerContext,
) {
    use surge_ts_diagnostics::DiagnosticCode;
    let (own_span, error_span) = match expression {
        ParsedExpression::ArrowFunction(function) => (function.span, function.error_span),
        ParsedExpression::ClassExpression(class) => (None, class.error_span),
        // `getEffectiveCheckNode` looks through `satisfies` and parentheses.
        ParsedExpression::SatisfiesExpression { .. } => {
            (None, satisfies_operand(expression).and_then(|(_, span)| span))
        }
        _ => return,
    };
    let Some(error_span) = error_span else {
        return;
    };
    let wholes: Vec<_> = [own_span, fallback_span]
        .into_iter()
        .flatten()
        .filter(|whole| *whole != error_span)
        .map(crate::context::convert_span)
        .collect();
    if wholes.is_empty() {
        return;
    }
    for diagnostic in &mut ctx.diagnostics[before..] {
        if matches!(
            diagnostic.code,
            DiagnosticCode::TypeScript(2322 | 2345 | 2769 | 1360 | 2741 | 2739 | 2740 | 2559 | 2375 | 2379)
        ) && diagnostic.span.as_ref().is_some_and(|span| wholes.contains(span))
        {
            diagnostic.span = Some(crate::context::convert_span(error_span));
        }
    }
}

fn evaluate_anchored(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_type: Option<&Type>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    // A call argument inside a returned value is related to its parameter,
    // fresh, wherever the call is, and what that reports is the call's, not a
    // verdict on the return.
    let outer_return_check = (expected_diagnostic == ExpectedTypeDiagnostic::ArgumentNotAssignable).then(|| {
        (
            std::mem::replace(&mut ctx.relating_unfresh_return, false),
            std::mem::replace(&mut ctx.in_contextual_return_check, false),
        )
    });
    // The flag is read by the next arrow checked, so an argument that is
    // itself a function, checked before the callee, would take it.
    let iife = expected_type.is_some()
        && matches!(
            expression,
            ParsedExpression::ExpressionCall { callee, arguments, .. }
                if matches!(callee.as_ref(), ParsedExpression::ArrowFunction(_))
                    && !arguments
                        .iter()
                        .any(|argument| matches!(argument.expression, ParsedExpression::ArrowFunction(_)))
        );
    if iife {
        crate::checks::function::NEXT_ARROW_IIFE_CONTEXTUAL.set(true);
    }
    let result = evaluate_contextually(
        expression,
        fallback_span,
        target_span,
        expected_type,
        expected_diagnostic,
        symbols,
        ctx,
    );
    if iife {
        crate::checks::function::NEXT_ARROW_IIFE_CONTEXTUAL.set(false);
    }
    if let Some((unfresh, return_check)) = outer_return_check {
        ctx.relating_unfresh_return = unfresh;
        ctx.in_contextual_return_check = return_check;
    }
    if let InferredExpression::Known(ty) = &result {
        crate::semantic::record_expression_type(expression, ty, ctx);
    }
    result
}

fn evaluate_contextually(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_type: Option<&Type>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    // A degraded expectation carries no contextual parameter types, so any
    // callback or method written against it would be reported implicit-any for
    // a shape surge failed to model rather than one the source omits.
    // An *error* type is not a modelling gap — tsc has none either, so it keeps
    // reporting through it: `getContextualSignature` finds nothing on
    // `errorType`, which leaves a callback's parameters implicit `any`
    // (TS7006). The expectation still carries nothing to type against, so the
    // evaluation is the degraded one; only the suppression depth is withheld.
    if expected_type.is_some_and(expectation_is_error_type) {
        return evaluate_expression_with_expected_type_inner(
            expression,
            fallback_span,
            target_span,
            Some(&Type::Unknown),
            expected_diagnostic,
            symbols,
            ctx,
        );
    }
    // A type variable of the body being checked is a template literal
    // context for a template expression like any string-like target.
    if let Some(expected) = expected_type.filter(|expected| expected.is_type_variable())
        && let Some(template) = contextual_template_literal_type(expression, expected, symbols, ctx)
    {
        let _ = evaluate_expression(expression, fallback_span, symbols, ctx);
        return InferredExpression::Known(template);
    }
    if expected_type.is_some_and(expectation_is_degraded) {
        ctx.degraded_expected_type_depth += 1;
        let result = evaluate_expression_with_expected_type_inner(
            expression,
            fallback_span,
            target_span,
            Some(&Type::Unknown),
            expected_diagnostic,
            symbols,
            ctx,
        );
        ctx.degraded_expected_type_depth -= 1;
        return result;
    }
    // An intersection that lost an operand keeps the survivor but marks it open.
    // The members it could not enumerate are exactly the ones a callback would
    // have been contextually typed by — tRPC's handler options are
    // `HTTPBaseHandlerOptions & CreateContextCallback<…> & { … }`, and the
    // middle operand is a conditional surge cannot decide — so an implicit-any
    // reported inside such a literal describes the dropped operand, not the
    // source. The expectation itself is kept: the properties surge *did*
    // enumerate still type their values.
    if expected_type.is_some_and(expectation_lost_an_operand) {
        ctx.degraded_expected_type_depth += 1;
        let result = evaluate_expression_with_expected_type_inner(
            expression,
            fallback_span,
            target_span,
            expected_type,
            expected_diagnostic,
            symbols,
            ctx,
        );
        ctx.degraded_expected_type_depth -= 1;
        return result;
    }

    evaluate_expression_with_expected_type_inner(
        expression,
        fallback_span,
        target_span,
        expected_type,
        expected_diagnostic,
        symbols,
        ctx,
    )
}

/// Whether the expectation is an object an intersection merge left *open*
/// because one of its operands could not be modelled. Unlike the degradation
/// sentinel this still carries real properties, so it stays the expectation and
/// only the implicit-any reporting is suppressed.
fn expectation_lost_an_operand(expected_type: &Type) -> bool {
    match expected_type {
        Type::Object(object) => object.synthetic_open_index,
        Type::Reference(_) => match expected_type.peeled() {
            Type::Object(object) => object.synthetic_open_index,
            _ => false,
        },
        _ => false,
    }
}

/// Whether the expectation is the degradation sentinel, directly or as a member
/// of a union.
///
/// A union carrying the sentinel is not a narrower expectation than the sentinel
/// alone — it is the same "surge does not know", with one shape it happened to
/// resolve. Contextual typing must not pick that shape: merging zod's two
/// `tuple` overloads yields `unknown | []`, and selecting the empty tuple made
/// every `z.tuple([a, b])` report `Type '[A, B]' is not assignable to type '[]'`.
/// A *written* `unknown` is [`Type::GenuineUnknown`], so a real annotation never
/// lands here.
/// Whether the expectation is the *error* type — a name or module the source
/// itself does not resolve, which tsc models as `errorType`. Distinct from
/// [`expectation_is_degraded`], which is surge's own "could not model this".
fn expectation_is_error_type(expected_type: &Type) -> bool {
    match expected_type {
        Type::ErrorType => true,
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| matches!(member, Type::ErrorType)),
        _ => false,
    }
}

fn expectation_is_degraded(expected_type: &Type) -> bool {
    match expected_type {
        Type::Unknown | Type::ErrorType | Type::TypeParameter(_) => true,
        // A deferred reference whose expansion is the sentinel carries no more
        // than the sentinel does.
        Type::Reference(_) => matches!(expected_type.peeled(), Type::Unknown),
        // A type variable of the body being checked is a member like any
        // other; the rest of the union still types the expression.
        Type::Union(union) => union.types().iter().any(|member| {
            matches!(member, Type::Unknown | Type::ErrorType)
                || matches!(member, Type::TypeParameter(parameter) if !parameter.is_active_variable())
        }),
        _ => false,
    }
}

fn evaluate_expression_with_expected_type_inner(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_type: Option<&Type>,
    _expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let Some(expected_type) = expected_type else {
        return evaluate_expression(expression, fallback_span, symbols, ctx);
    };
    let _written_type_arguments = crate::checks::call::WrittenTypeArguments::enter(expression);
    // `getContextualSignature` finds none in `any`, the error type, or the
    // global `Function` (no call signature).
    if (matches!(expected_type, Type::Any | Type::ErrorType) || is_global_function_type(expected_type))
        && _expected_diagnostic == ExpectedTypeDiagnostic::ArgumentNotAssignable
        && let ParsedExpression::ArrowFunction(arrow) = expression
        && arrow.callback_argument
    {
        crate::checks::function::NEXT_ARROW_CONTEXTUAL_ANY.set(true);
        let result = evaluate_expression(expression, fallback_span, symbols, ctx);
        crate::checks::function::NEXT_ARROW_CONTEXTUAL_ANY.set(false);
        return result;
    }
    // `checkYieldExpression`: a `yield` whose contextual type is `any` is as
    // implicit as one with none.
    if let ParsedExpression::Yield { delegate: false, span: Some(span), .. } = expression
        && matches!(expected_type, Type::Any)
        && ctx.implicit_any_yield_generator
        && ctx.in_generator_function
        && !ctx.implicit_any_yields.contains(&span.start)
    {
        ctx.push(crate::spans::diagnostic_with_syntax_span(
            surge_ts_diagnostics::Diagnostic::ts7057(ctx.file_name.clone()),
            Some(crate::checks::expr::yield_keyword_span(*span)),
        ));
    }

    // tsc relates a value through `satisfies` (`getEffectiveCheckNode`): a
    // literal inside elaborates a mismatch at its members, anything else is
    // reported at the operand itself.
    if matches!(
        _expected_diagnostic,
        ExpectedTypeDiagnostic::ArgumentNotAssignable | ExpectedTypeDiagnostic::TypeNotAssignable
    ) && let Some((satisfied, satisfied_span)) = satisfies_operand(expression)
        // Outside an argument the literal stays contextually typed by the
        // `satisfies` type alone.
        && (_expected_diagnostic == ExpectedTypeDiagnostic::ArgumentNotAssignable
            || !matches!(satisfied, ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }))
    {
        let _ = evaluate_expression(expression, fallback_span, symbols, ctx);
        return evaluate_expression_with_expected_type_anchored(
            satisfied,
            satisfied_span.or(fallback_span),
            target_span,
            Some(expected_type),
            _expected_diagnostic,
            symbols,
            ctx,
        );
    }

    // `checkAssignmentOperator` yields the right operand's type, freshness
    // included, so an object literal assigned in a value position
    // (`a = b = { x, y }`, `return o = { x, y }`) is excess-checked against the
    // outer target too. The excess report precedes the structural comparison,
    // weak-type detection included; a written member that fails its target
    // property is elaborated by the comparison instead.
    if let ParsedExpression::Assignment { value, .. } = expression
        && let ParsedExpression::ObjectLiteral { properties, span } = value.as_ref()
    {
        let before = ctx.diagnostics().len();
        let result = evaluate_expression(expression, fallback_span, symbols, ctx);
        if ctx.diagnostics().len() == before
            && let InferredExpression::Known(value_type) = &result
            && let Type::Object(source) = value_type.peeled()
            && !crate::checks::function::type_contains_degradation(expected_type)
            && let Type::Object(target) = expected_type.peeled()
            && source.properties.iter().all(|(name, property)| {
                target.get_property(name).is_none_or(|declared| {
                    let declared_type = if declared.optional {
                        surge_ts_types::union_type(vec![declared.ty.clone(), Type::Undefined])
                    } else {
                        declared.ty.clone()
                    };
                    is_assignable_to(&property.ty, &declared_type)
                })
            })
            && report_excess_property(properties, &target, choose_span(*span, fallback_span), symbols, ctx)
        {
            return InferredExpression::Known(expected_type.clone());
        }
        return result;
    }

    // Before the expected type is peeled below: a template literal type
    // resolves to `string`, and the pattern is what this reads.
    if let Some(template) = contextual_template_literal_type(expression, expected_type, symbols, ctx)
    {
        // The interpolations are still checked as expressions; only the type
        // the template evaluates to changes.
        let _ = evaluate_expression(expression, fallback_span, symbols, ctx);
        return InferredExpression::Known(template);
    }

    // `new C(...)` contextually typed by a generic instance reference (e.g.
    // `Promise<void>`) lets the constructor infer its type arguments. Route it
    // through `check_new_like` with the (un-peeled) expected reference so the
    // executor's callback parameters are typed.
    if let ParsedExpression::New {
        callee,
        callee_span,
        span,
        type_arguments,
        arguments,
        ..
    } = expression
    {
        return match super::call::check_new_like(
            callee,
            *callee_span,
            *span,
            type_arguments,
            arguments,
            Some(expected_type),
            symbols,
            ctx,
        ) {
            Some(result_type) => InferredExpression::Known(result_type),
            None => InferredExpression::Unknown,
        };
    }

    // `getContextualType` hands a non-null expression its parent's contextual
    // type, so a generic call under `!` infers from it as it would bare:
    // `const r: SVGRectElement = document.querySelector(s)!`.
    if let ParsedExpression::NonNullAssertion {
        expression: asserted,
        span,
        in_optional_chain: false,
    } = expression
        && matches!(
            asserted.as_ref(),
            ParsedExpression::Call { .. } | ParsedExpression::PropertyCall { .. }
        )
    {
        return match evaluate_expression_with_expected_type_inner(
            asserted,
            span.or(fallback_span),
            target_span,
            Some(expected_type),
            _expected_diagnostic,
            symbols,
            ctx,
        ) {
            InferredExpression::Known(ty) => InferredExpression::Known(surge_ts_types::remove_nullish(&ty)),
            other => other,
        };
    }

    // A generic call whose type parameter occurs only in the return type
    // (`const c: Ctor<MyZ> = make("x", (inst) => …)`) can infer it from the
    // contextual type alone. Route the call through the expected-type entry with
    // the *un-peeled* expected type, so the inference matches type argument for
    // type argument against the declared return reference.
    if let ParsedExpression::Call {
        callee_name,
        callee_span,
        type_arguments,
        arguments,
        ..
    } = expression
    {
        return match super::call::check_call_like_with_expected_type(
            callee_name,
            *callee_span,
            super::call::call_span_within(*callee_span, fallback_span),
            type_arguments,
            arguments,
            Some(expected_type),
            symbols,
            ctx,
        ) {
            Some(return_type) => InferredExpression::Known(return_type),
            None => InferredExpression::Unknown,
        };
    }

    // `const xs: T = await Promise.all(…)`: the awaited call is contextually
    // typed by `T` (tsc hands an `await` operand `T | PromiseLike<T>`), which is
    // what lets `Promise.all` tell a result the context would have shaped
    // differently from one it can assert, and a call returning `Promise<R>`
    // infer `R`.
    if let ParsedExpression::Await { operand, operand_span, .. } = expression
        && let ParsedExpression::PropertyCall { property_name, .. } = operand.as_ref()
        && property_name == "all"
    {
        return match evaluate_expression_with_expected_type(
            operand,
            operand_span.or(fallback_span),
            Some(expected_type),
            ExpectedTypeDiagnostic::TypeNotAssignable,
            symbols,
            ctx,
        ) {
            InferredExpression::Known(ty) => {
                InferredExpression::Known(crate::checks::call::awaited_type(&ty))
            }
            other => other,
        };
    }
    if let ParsedExpression::Await { operand, operand_span, .. } = expression
        && matches!(operand.as_ref(), ParsedExpression::Call { .. } | ParsedExpression::PropertyCall { .. })
    {
        let awaited_expected = crate::checks::call::awaited_type(expected_type);
        return match crate::checks::call::with_awaited_context(&awaited_expected, || {
            evaluate_expression_with_expected_type(
                operand,
                operand_span.or(fallback_span),
                Some(&awaited_expected),
                ExpectedTypeDiagnostic::TypeNotAssignable,
                symbols,
                ctx,
            )
        }) {
            InferredExpression::Known(ty) => {
                InferredExpression::Known(crate::checks::call::awaited_type_with_alias(&ty, ctx))
            }
            other => other,
        };
    }

    // The same contextual inference for a call through a member. tsc makes no
    // distinction — `inferTypeArguments` reads `getContextualType(node)` for
    // whatever the callee is — but surge reached it only from the bare-callee
    // branch above, so `const c: core.Ctor<MyZ> = core.make("x", (inst) => …)`
    // inferred nothing and the callback's parameters had no type at all.
    if let ParsedExpression::PropertyCall {
        object,
        object_span,
        property_name,
        property_span,
        call_span,
        type_arguments,
        arguments,
        ..
    } = expression
    {
        return match super::call::check_property_call_like(
            object,
            *object_span,
            property_name,
            *property_span,
            *call_span,
            type_arguments,
            arguments,
            Some(expected_type),
            symbols,
            ctx,
        ) {
            Some(return_type) => InferredExpression::Known(return_type),
            None => InferredExpression::Unknown,
        };
    }
    if let ParsedExpression::OptionalPropertyCall {
        object,
        object_span,
        property_name,
        property_span,
        call_span,
        type_arguments,
        arguments,
    } = expression
    {
        return match super::call::check_optional_property_call(
            object,
            *object_span,
            property_name,
            *property_span,
            *call_span,
            type_arguments,
            arguments,
            Some(expected_type),
            symbols,
            ctx,
        ) {
            Some(return_type) => InferredExpression::Known(return_type),
            None => InferredExpression::Unknown,
        };
    }

    // A generic expected type (`Props`, `Box<T>`, …) is a nominal
    // `Type::Reference`; peel it to its structural shape so the contextual-typing
    // dispatch below (function/tuple/array/object/union) sees the real form
    // instead of falling through to context-free evaluation.
    // Under `exactOptionalPropertyTypes` an optional element's slot carries no
    // `undefined`, so only the recorded `minLength` says it may be absent.
    let exact_optional_min_length = (surge_ts_types::strict_null_checks()
        && surge_ts_types::exact_optional_property_types())
    .then(|| surge_ts_types::fixed_tuple_parts(expected_type).map(|(_, min_length)| min_length))
    .flatten();
    let written_expected = expected_type;
    let peeled_expected;
    let expected_type = match expected_type {
        Type::Reference(reference) => {
            peeled_expected = reference.resolve().peeled();
            &peeled_expected
        }
        other => other,
    };

    // The contextual type reaches the initializers of the class's static
    // properties (`getContextualTypeForStaticPropertyDeclaration`).
    if let ParsedExpression::ClassExpression(class_expression) = expression {
        return InferredExpression::Known(crate::program::check_class_expression(
            class_expression,
            Some(expected_type),
            symbols,
            ctx,
        ));
    }

    if let (Type::Function(expected_function_type), ParsedExpression::ArrowFunction(arrow)) =
        (expected_type, expression)
    {
        // Several call signatures (an overload group, or an intersection of
        // callable types) type the function from their combination
        // (`getContextualCallSignature`).
        let intersected = expected_function_type
            .overloads()
            .is_some()
            .then(|| intersected_call_signature(expected_function_type, arrow, ctx))
            .flatten();
        let expected_function_type = intersected.as_ref().unwrap_or(expected_function_type);
        ctx.next_arrow_is_argument =
            matches!(_expected_diagnostic, ExpectedTypeDiagnostic::ArgumentNotAssignable);
        ctx.next_arrow_context_only =
            matches!(_expected_diagnostic, ExpectedTypeDiagnostic::ContextOnly);
        let function_type = crate::checks::function::check_arrow_function_expression_anchored(
            with_type_copy_reason(TypeCopyReason::ExpectedType, || arrow.as_ref().clone()),
            Some(expected_function_type),
            target_span,
            symbols,
            ctx,
        );
        return InferredExpression::Known(Type::Function(function_type));
    }

    // A callable object expected type — an interface carrying a call signature, such
    // as React's `ForwardRefRenderFunction` — contextually types an arrow the same
    // way a bare function type does, so its parameters are not implicit-any.
    if let (Type::Object(expected_object), ParsedExpression::ArrowFunction(arrow)) =
        (expected_type, expression)
    {
        // An intersection's members each contribute a call signature, which
        // combine into one (`getIntersectedSignatures`) or give none.
        let intersected = expected_object
            .call_signature()
            .filter(|head| head.overloads().is_some())
            .and_then(|head| intersected_call_signature(head, arrow, ctx));
        let call_signature = intersected.as_ref().or(expected_object.call_signature());
        if let Some(call_signature) = call_signature {
            ctx.next_arrow_is_argument =
                matches!(_expected_diagnostic, ExpectedTypeDiagnostic::ArgumentNotAssignable);
            ctx.next_arrow_context_only =
                matches!(_expected_diagnostic, ExpectedTypeDiagnostic::ContextOnly);
            let function_type = crate::checks::function::check_arrow_function_expression_anchored(
                with_type_copy_reason(TypeCopyReason::ExpectedType, || arrow.as_ref().clone()),
                Some(call_signature),
                target_span,
                symbols,
                ctx,
            );
            return InferredExpression::Known(Type::Function(function_type));
        }
    }

    // A function contextually typed by a union (`Hook | Hook[]`, the shape hook
    // options take) draws its signature from the union's single callable member,
    // mirroring tsc's `getContextualSignature`. Without it the arrow is evaluated
    // context-free and its parameters become implicit any (false TS7006). A union
    // with several callable members stays context-free: picking one there needs
    // signature matching and guessing wrong types the parameters as the wrong shape.
    if let (Type::Union(union), ParsedExpression::ArrowFunction(_)) = (expected_type, expression) {
        let callable: Vec<&Type> = union
            .types()
            .iter()
            .filter(|member| is_contextual_callable(member))
            .collect();
        if let [member] = callable.as_slice() {
            let member = with_type_copy_reason(TypeCopyReason::ExpectedType, || (*member).clone());
            return evaluate_expression_with_expected_type_anchored(
                expression,
                fallback_span,
                target_span,
                Some(&member),
                _expected_diagnostic,
                symbols,
                ctx,
            );
        }
        // Several callable members of the same arity. tsc never sees this
        // union: it resolves the overload first and contextually types the
        // arrow from the one it picked. surge merges overloads into a single
        // permissive signature instead, so the union is its own artifact and an
        // implicit-any report here would describe that gap rather than the
        // source (`ws.addEventListener("close", (event) => …)`, whose merged
        // listener slot holds both the generic overload's callback and
        // `EventListenerOrEventListenerObject`). Members of *differing* arity
        // are a genuinely ambiguous union, which tsc also refuses to type — the
        // implicit-any there is real, so it still reports.
        // `getContextualSignature` over a union: members whose signatures are
        // identical but for their return types combine into one
        // (`createUnionSignature`) that returns the union of theirs. Only an
        // expression body takes its own return type from under it; a block
        // body keeps the contextual one, which no single member accepts.
        if callable.len() > 1
            && let ParsedExpression::ArrowFunction(arrow) = expression
            && matches!(arrow.body, surge_ts_syntax::ParsedArrowFunctionBody::Expression(_))
        {
            let signatures: Vec<surge_ts_types::FunctionType> =
                callable.iter().filter_map(|member| contextual_call_signature(member)).collect();
            if let Some(first) = signatures.first()
                && signatures.len() == callable.len()
                && signatures.iter().all(|signature| {
                    signature.type_parameter_head().is_none()
                        && signature.overloads().is_none()
                        && signature.parameters() == first.parameters()
                        && signature.required_parameter_count() == first.required_parameter_count()
                        && signature.is_variadic() == first.is_variadic()
                })
            {
                let combined = first.with_signature_types(
                    first.parameters().to_vec(),
                    surge_ts_types::union_type(
                        signatures.iter().map(|signature| signature.return_type().clone()).collect(),
                    ),
                );
                return evaluate_expression_with_expected_type_anchored(
                    expression,
                    fallback_span,
                    target_span,
                    Some(&Type::Function(combined)),
                    _expected_diagnostic,
                    symbols,
                    ctx,
                );
            }
        }
        // An argument's union may be an overload fold; a declaration's or an
        // assignment's is written.
        if callable.len() > 1
            && callable_members_share_arity(&callable)
            && !(matches!(_expected_diagnostic, ExpectedTypeDiagnostic::TypeNotAssignable)
                && written_function_union(&callable))
        {
            ctx.degraded_expected_type_depth += 1;
            let result = evaluate_expression(expression, fallback_span, symbols, ctx);
            ctx.degraded_expected_type_depth -= 1;
            return result;
        }
    }

    if let ParsedExpression::ConstAssertion {
        expression: inner,
        span: assertion_span,
    } = expression
    {
        let checked = evaluate_expression_with_expected_type_anchored(
            inner,
            fallback_span,
            target_span,
            Some(expected_type),
            _expected_diagnostic,
            symbols,
            ctx,
        );
        // The context only reaches the operand: `as const` types it as its
        // readonly literal form whatever is expected of it.
        if !matches!(checked, InferredExpression::Known(_)) {
            return checked;
        }
        let before = ctx.diagnostics().len();
        let asserted = crate::checks::expr::evaluate_const_expression(
            inner,
            assertion_span.or(fallback_span),
            symbols,
            ctx,
        );
        ctx.truncate_diagnostics_releasing_utility_keys(before);
        return match asserted {
            asserted @ InferredExpression::Known(_) => asserted,
            _ => checked,
        };
    }

    // `fallback ?? (…)` and `fallback || (…)` produce the whole expression's
    // value from either operand, so tsc hands the contextual type to the right
    // one just as it does to a ternary's branches. Without this an arrow written
    // as the default (`opts.onSuccess ?? ((options) => …)`) is evaluated
    // context-free and its parameters become implicit any (false TS7006). The
    // left operand keeps driving its own narrowing and stays context-free.
    if let ParsedExpression::NullishCoalescing {
        left,
        left_span,
        right,
        right_span,
    } = expression
    {
        let left_result = evaluate_expression(left, left_span.or(fallback_span), symbols, ctx);
        let literal_right = is_fresh_literal_operand(right);
        let right_result = evaluate_expression_with_expected_type_anchored(
            right,
            right_span.or(fallback_span),
            target_span,
            Some(expected_type),
            if literal_right { ExpectedTypeDiagnostic::ContextOnly } else { _expected_diagnostic },
            symbols,
            ctx,
        );
        if literal_right {
            let own_right = literal_operand_own_type(right, &right_result, right_span.or(fallback_span), symbols, ctx);
            let related = join_nullish_coalescing(left_result.clone(), own_right);
            relate_joined_operand_value(&related, expected_type, target_span.or(fallback_span), _expected_diagnostic, ctx);
        }
        return join_nullish_coalescing(left_result, right_result);
    }

    if let ParsedExpression::Logical {
        left,
        left_span,
        operator: operator @ surge_ts_syntax::ParsedLogicalOperator::Or,
        right,
        right_span,
        ..
    } = expression
    {
        let mut left_result = evaluate_expression(left, left_span.or(fallback_span), symbols, ctx);
        // A fresh literal that never falls through is the whole value, and is
        // checked as one, excess properties included.
        let literal_is_value = matches!(
            left.as_ref(),
            ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }
        ) && matches!(&left_result, InferredExpression::Known(left_ty)
            if crate::checks::ops::logical_or_left_never_falsy(left_ty));
        if literal_is_value {
            left_result = evaluate_expression_with_expected_type_anchored(
                left,
                left_span.or(fallback_span),
                target_span,
                Some(expected_type),
                _expected_diagnostic,
                symbols,
                ctx,
            );
        }
        let right_symbols = crate::checks::expr::logical_right_operand_symbols(left, operator, symbols, ctx);
        // tsc relates the whole value (`elaborateError` never enters `||`):
        // a literal right operand only takes the contextual type.
        let literal_right = is_fresh_literal_operand(right);
        let right_result = evaluate_expression_with_expected_type_anchored(
            right,
            right_span.or(fallback_span),
            target_span,
            Some(expected_type),
            if literal_right || literal_is_value { ExpectedTypeDiagnostic::ContextOnly } else { _expected_diagnostic },
            right_symbols.as_ref().unwrap_or(symbols),
            ctx,
        );
        if literal_right && !literal_is_value {
            let own_right = literal_operand_own_type(
                right,
                &right_result,
                right_span.or(fallback_span),
                right_symbols.as_ref().unwrap_or(symbols),
                ctx,
            );
            let related = crate::checks::ops::evaluate_logical_expression(*operator, left_result.clone(), own_right);
            relate_joined_operand_value(&related, expected_type, target_span.or(fallback_span), _expected_diagnostic, ctx);
        }
        return crate::checks::ops::evaluate_logical_expression(*operator, left_result, right_result);
    }

    // tsc's `getContextualTypeForBinaryOperand`: the right operand of `&&`
    // (and so of `&&=`) is contextually typed by the whole expression's
    // contextual type, the left not at all. The result is the union with the
    // left's falsy part, so the operand only takes the type as context.
    if let ParsedExpression::Logical {
        left,
        left_span,
        operator: operator @ surge_ts_syntax::ParsedLogicalOperator::And,
        right,
        right_span,
        truthiness_tests,
        ..
    } = expression
    {
        crate::checks::function::report_unreferenced_callable_conditions(
            truthiness_tests,
            expression,
            symbols,
            ctx,
        );
        return crate::checks::expr::evaluate_logical_in_context(
            left,
            left_span,
            operator,
            right,
            right_span,
            fallback_span,
            Some(expected_type),
            symbols,
            ctx,
        );
    }

    // A comma expression's value is its last operand, which is the one the
    // contextual type reaches.
    if let ParsedExpression::Sequence { expressions } = expression
        && let Some(((last, last_span), leading)) = expressions.split_last()
    {
        for (operand, span) in leading {
            let _ = evaluate_expression(operand, span.or(fallback_span), symbols, ctx);
        }
        return evaluate_expression_with_expected_type_anchored(
            last,
            last_span.or(fallback_span),
            target_span,
            Some(written_expected),
            _expected_diagnostic,
            symbols,
            ctx,
        );
    }

    // Each branch is related to the type as written: the peeled form of
    // `readonly T[]` is the mutable array, which a readonly branch never fits.
    if matches!(expression, ParsedExpression::Conditional { .. }) {
        return evaluate_conditional_expression_with_expected_type(
            expression,
            fallback_span,
            written_expected,
            _expected_diagnostic,
            symbols,
            ctx,
        );
    }

    // A spread changes what the literal *is*: tsc's `checkArrayLiteral` flattens
    // a spread tuple into its elements and turns a spread array into a rest
    // slot, so `[1, ...pair]` is `[number, string, string]` and `[1, ...strs]`
    // is `[number, ...string[]]`. Checking the spread as one element against
    // one slot compared `[string, string]` with `string`.
    if let (Type::Tuple(_) | Type::OpenTuple(_), ParsedExpression::ArrayLiteral { elements, .. }) =
        (expected_type, expression)
        && elements.iter().any(|element| element.spread)
    {
        return evaluate_spread_tuple_literal(elements, expected_type, false, symbols, ctx);
    }

    // An argument after a call's first failing one only types its elements —
    // their callbacks' parameters — and relates none of them.
    if _expected_diagnostic == ExpectedTypeDiagnostic::ContextOnly
        && ctx.suppressed_argument_mismatch_span.is_some()
        && matches!(expected_type, Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_))
        && let ParsedExpression::ArrayLiteral { elements, .. } = expression
        && !elements.is_empty()
        && elements.iter().all(|element| !element.spread)
    {
        return evaluate_array_literal_in_context(elements, expected_type, fallback_span, symbols, ctx);
    }

    if let (Type::Tuple(expected_elements), ParsedExpression::ArrayLiteral { elements, span, .. }) =
        (expected_type, expression)
    {
        return evaluate_tuple_literal_with_expected_type(
            elements,
            expected_elements,
            exact_optional_min_length,
            choose_span(*span, fallback_span),
            target_span,
            matches!(_expected_diagnostic, ExpectedTypeDiagnostic::ArgumentNotAssignable),
            symbols,
            ctx,
        );
    }

    // A tuple with a rest slot is tuple-like too, so tsc's `checkArrayLiteral`
    // types the literal as a tuple against it rather than as an array — which
    // is what lets `[1, "a", "b"]` satisfy `[number, ...string[]]` when
    // `(number | string)[]` does not. A spread element has no single slot and
    // goes to the generic path.
    if let (Type::OpenTuple(open), ParsedExpression::ArrayLiteral { elements, span, .. }) =
        (expected_type, expression)
        && elements.iter().all(|element| !element.spread)
    {
        return evaluate_open_tuple_literal_with_expected_type(
            elements,
            open,
            expected_type,
            choose_span(*span, fallback_span),
            symbols,
            ctx,
        );
    }

    if let (Type::Array(expected_element_type), ParsedExpression::ArrayLiteral { elements, span, .. }) =
        (expected_type, expression)
    {
        return evaluate_array_literal_with_expected_type(
            elements,
            expected_element_type,
            choose_span(*span, fallback_span),
            symbols,
            ctx,
        );
    }

    if let ParsedExpression::ArrayLiteral { elements, .. } = expression
        && is_tuple_like_object_target(expected_type)
    {
        return evaluate_spread_tuple_literal(elements, expected_type, true, symbols, ctx);
    }

    // `elaborateArrayLiteral` against an object with a numeric index signature
    // (`ArrayLike<T>`): each element is related to the index type, and an
    // array of it is what the caller relates to the rest of the object.
    if let ParsedExpression::ArrayLiteral { elements, span, .. } = expression
        && let Type::Object(object) = expected_type.peeled()
        && object.string_index_type.is_none()
        && object.call_signature().is_none()
        && object.construct_signature().is_none()
        && !object.is_intersection
        && let Some(index_type) = object.number_index_type.as_deref()
        && !index_type.is_unmodelled()
        && !crate::checks::function::type_contains_degradation(index_type)
    {
        return evaluate_array_literal_with_expected_type(
            elements,
            index_type,
            choose_span(*span, fallback_span),
            symbols,
            ctx,
        );
    }

    // `getContextualTypeForElementExpression`: against an iterable that is no
    // array, each element is contextually typed by the iterated type. No
    // element has a slot to be elaborated onto, so the literal is related as
    // a whole by the caller.
    if let ParsedExpression::ArrayLiteral { elements, .. } = expression
        && !elements.is_empty()
        && elements.iter().all(|element| !element.spread)
        && matches!(expected_type.peeled(), Type::Object(_))
        && let Some(iterated) = crate::checks::expr::iteration_types_of_iterable(
            expected_type,
            crate::checks::expr::IterationUse::sync(false),
        )
        .map(|types| types.yield_type)
        .filter(|ty| !ty.is_unmodelled())
    {
        return evaluate_array_literal_in_context(
            elements,
            &Type::Array(Box::new(iterated)),
            fallback_span,
            symbols,
            ctx,
        );
    }

    // An object literal against a tuple or array target is checked against its
    // apparent members (indices, `length`, the array methods): tsc reports the
    // first property that is none of them as excess, at the property, rather
    // than the whole literal as unassignable.
    // A function type declares no members at all (`isKnownProperty` reads
    // only its own), so every written property is excess.
    let function_target = matches!(expected_type, Type::Function(_))
        && _expected_diagnostic != ExpectedTypeDiagnostic::ContextOnly;
    if let (Type::Tuple(_) | Type::Array(_) | Type::Function(_), ParsedExpression::ObjectLiteral { properties, span }) =
        (expected_type, expression)
        && (function_target || !matches!(expected_type, Type::Function(_)))
        && !ctx.relating_unfresh_return
        && let Some(property) = properties.iter().find(|property| {
            !property.is_spread
                && (function_target
                    || expected_type
                        .get_property_access_type(&property.name)
                        .is_none())
        })
    {
        let diagnostic =
            Diagnostic::ts2353(written_object_property_name(property), &expected_type.name(), ctx.file_name.clone());
        ctx.push(diagnostic_with_syntax_span(
            diagnostic,
            choose_span(
                property.name_span,
                choose_span(property.span, choose_span(*span, fallback_span)),
            ),
        ));
        return InferredExpression::Unknown;
    }

    // `elaborateError` declines a target that is or has a generic conditional
    // type (`isOrHasGenericConditional`): the literal is typed in its context
    // and related as a whole, which such a conditional may refuse though every
    // member fits.
    if let (
        Type::Object(expected_object_type),
        ParsedExpression::ObjectLiteral { properties, span },
    ) = (expected_type, expression)
        && _expected_diagnostic != ExpectedTypeDiagnostic::ContextOnly
        && has_generic_conditional_operand(expected_object_type)
    {
        let _ = evaluate_object_literal_with_expected_type(
            properties,
            expected_object_type,
            choose_span(*span, fallback_span),
            target_span,
            ExpectedTypeDiagnostic::ContextOnly,
            symbols,
            ctx,
        );
        return crate::infer::infer_expression(expression, symbols, ctx);
    }

    if let (
        Type::Object(expected_object_type),
        ParsedExpression::ObjectLiteral { properties, span },
    ) = (expected_type, expression)
    {
        return evaluate_object_literal_with_expected_type(
            properties,
            expected_object_type,
            choose_span(*span, fallback_span),
            target_span,
            _expected_diagnostic,
            symbols,
            ctx,
        );
    }

    // An object literal against a union target (an overload group's merged
    // parameter, `A | B`) is contextually typed by the member that declares the
    // most of the written properties, mirroring how tsc picks the overload the
    // argument fits. Without it the literal is evaluated context-free and its
    // method/callback parameters lose their types (false TS7006).
    // An array literal against a union target takes the union's lone array/tuple
    // member as its contextual type, so the elements are checked against the
    // element type rather than widened context-free (`items: [{ type: "string" }]`
    // against `_JSONSchema | _JSONSchema[]` widened the literal's `type` to
    // `string` and then rejected it).
    if let (Type::Union(union), ParsedExpression::ArrayLiteral { .. }) = (expected_type, expression)
    {
        // A `readonly T[]` member contextually types the literal exactly like
        // `T[]` does; the modifier only matters for the assignability test.
        let members: Vec<Type> = flattened_union_members(union)
            .iter()
            .map(mutable_sequence_shape)
            .collect();
        let mut array_members = members
            .iter()
            .filter(|member| matches!(member, Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_)));
        if let (Some(member), None) = (array_members.next(), array_members.next()) {
            return evaluate_expression_with_expected_type_anchored(
                expression,
                fallback_span,
                target_span,
                Some(member),
                _expected_diagnostic,
                symbols,
                ctx,
            );
        }
        // Several array/tuple members: the literal itself says which one it is,
        // the way a discriminated object literal does. `match<['+', number,
        // number] | ['-', number]>(['-', 2])` has to reach the `'-'` member, and
        // `[{ type: 'b', s: '2' }]` against `A[] | B[]` has to reach `B[]`, or
        // the literal is evaluated context-free and widens.
        if let ParsedExpression::ArrayLiteral { elements, .. } = expression
            && let Some(member) = sole_matching_sequence_member(&members, elements, symbols, ctx)
        {
            return evaluate_expression_with_expected_type_anchored(
                expression,
                fallback_span,
                target_span,
                Some(&member),
                _expected_diagnostic,
                symbols,
                ctx,
            );
        }
        // `checkArrayLiteral`'s tuple context: against a union of tuples none
        // of which takes the literal alone, it is still a tuple, each element
        // typed by the union of the members' slots, and it is related to the
        // union as a whole (`["a" | "b", number]` to `["a", number] | ["b",
        // number]` by its discriminant).
        if let ParsedExpression::ArrayLiteral { elements, .. } = expression
            && !elements.is_empty()
            && elements.iter().all(|element| !element.spread)
            && members.iter().all(|member| matches!(member, Type::Tuple(_)))
        {
            return evaluate_spread_tuple_literal(elements, expected_type, true, symbols, ctx);
        }
    }

    if let (Type::Union(union), ParsedExpression::ObjectLiteral { properties, .. }) =
        (expected_type, expression)
    {
        // The discriminant rule only ever *replaces* a report the machinery
        // below already made: which constituent a literal belongs to decides
        // how the failure reads, not whether there is one. A literal surge
        // accepts here is one tsc accepts too — a contextually typed callback's
        // returned literal is checked without freshness, so an excess key in it
        // is no error at all.
        let before = ctx.diagnostics().len();
        let result = object_literal_against_union_members(
            union,
            properties,
            expression,
            expected_type,
            fallback_span,
            target_span,
            _expected_diagnostic,
            symbols,
            ctx,
        );
        if ctx.diagnostics().len() > before
            && let Some((diagnostic, span)) = discriminant_replacement_report(
                union,
                properties,
                expression,
                expected_type,
                fallback_span,
                target_span,
                _expected_diagnostic,
                symbols,
                ctx,
            )
        {
            ctx.truncate_diagnostics_releasing_utility_keys(before);
            ctx.push(diagnostic_with_syntax_span(diagnostic, span));
        }
        if let Some(result) = result {
            return result;
        }
    }

    evaluate_expression_with_expected_type_rest(
        expression,
        fallback_span,
        target_span,
        expected_type,
        _expected_diagnostic,
        symbols,
        ctx,
    )
}

#[allow(clippy::too_many_arguments)]
/// A literal typed against the union member it belongs to may lack members of
/// that one constituent. tsc names those beneath the head, which stays the
/// relation to the union as written: `Argument of type '{ dog: string; }' is not
/// assignable to parameter of type 'ExoticAnimal'`.
fn rehead_missing_members_against_union(
    checkpoint: usize,
    expression: &ParsedExpression,
    expected_type: &Type,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    use surge_ts_diagnostics::DiagnosticCode;
    // `isRelatedTo` relates an object to a union of at most three members by
    // the one left once `null`/`undefined` are taken out, so that member is the
    // target the report names.
    if let Type::Union(union) = expected_type.peeled()
        && union.types().len() <= 3
        && union.types().iter().filter(|member| !matches!(member, Type::Undefined | Type::Null)).count() == 1
    {
        return;
    }
    let Some(index) = ctx.diagnostics[checkpoint..]
        .iter()
        .position(|diagnostic| {
            matches!(diagnostic.code, DiagnosticCode::TypeScript(2741 | 2739 | 2740))
        })
        .map(|offset| checkpoint + offset)
    else {
        return;
    };
    let diagnostics_before = ctx.diagnostics.len();
    let source = crate::infer::infer_expression(expression, symbols, ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    let InferredExpression::Known(source) = source else {
        return;
    };
    let source = crate::checks::expr::widen_type(&source);
    let is_argument = matches!(expected_diagnostic, ExpectedTypeDiagnostic::ArgumentNotAssignable);
    let span = ctx.diagnostics[index].span.clone();
    if is_argument && span.is_some() && span == ctx.suppressed_argument_mismatch_span {
        ctx.diagnostics.remove(index);
        return;
    }
    let (source_name, target_name) = (source.name(), expected_type.name());
    let replacement = if is_argument {
        Diagnostic::ts2345(&source_name, &target_name, ctx.file_name.clone())
    } else {
        Diagnostic::ts2322(&source_name, &target_name, ctx.file_name.clone())
    };
    let mut replacement = crate::checks::relation_chain::with_chain(
        replacement,
        &crate::checks::relation_chain::chain_beneath_head(&source, expected_type),
    );
    replacement.span = span;
    ctx.diagnostics[index] = replacement;
}

/// `elaborateElementwise` against a union target reads each property's target
/// as the union of what every member declares for it
/// (`getBestMatchIndexedAccessTypeOrUndefined`). A mismatch surge elaborated at
/// a property against the one member it picked, where the property fits that
/// union, is no elaboration tsc makes: the literal as a whole fails the union
/// instead, at the head.
fn rehead_unelaborated_union_properties(
    checkpoint: usize,
    properties: &[ParsedObjectProperty],
    members: &[Type],
    expression: &ParsedExpression,
    expected_type: &Type,
    target_span: Option<SyntaxTextSpan>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    use surge_ts_diagnostics::DiagnosticCode;
    if !matches!(expected_diagnostic, ExpectedTypeDiagnostic::TypeNotAssignable) || ctx.diagnostics.len() == checkpoint {
        return;
    }
    // Only a report on the property itself is its elaboration; one deeper in
    // its value (a callback body's return) is the value's own.
    let property_at = |diagnostic: &Diagnostic| {
        let start = diagnostic.span.as_ref()?.start;
        properties.iter().find(|property| {
            !property.is_spread
                && (property.name_span.is_some_and(|span| span.start == start)
                    || property.value_span.is_some_and(|span| span.start == start))
        })
    };
    let object_members = members
        .iter()
        .filter(|member| !matches!(member, Type::Undefined | Type::Null | Type::Void))
        .count();
    if object_members < 2 {
        return;
    }
    let elaborated: Vec<usize> = (checkpoint..ctx.diagnostics.len())
        .filter(|&index| matches!(ctx.diagnostics[index].code, DiagnosticCode::TypeScript(2322)))
        .filter(|&index| property_at(&ctx.diagnostics[index]).is_some())
        .collect();
    if elaborated.is_empty() || elaborated.len() != ctx.diagnostics.len() - checkpoint {
        return;
    }
    let diagnostics_before = ctx.diagnostics.len();
    let source = crate::infer::infer_expression(expression, symbols, ctx);
    ctx.truncate_diagnostics(diagnostics_before);
    let InferredExpression::Known(source) = source else {
        return;
    };
    let source = crate::checks::expr::widen_type(&source);
    for &index in &elaborated {
        let Some(property) = property_at(&ctx.diagnostics[index]) else {
            return;
        };
        let declared: Option<Vec<Type>> = members
            .iter()
            .filter(|member| !matches!(member, Type::Undefined | Type::Null | Type::Void))
            .map(|member| member.get_property_access_type(&property.name))
            .collect();
        let (Some(declared), Some(source_property)) = (declared, source.get_property_access_type(&property.name))
        else {
            return;
        };
        if !surge_ts_types::is_assignable_to(&source_property, &surge_ts_types::union_type(declared)) {
            return;
        }
    }
    let span = match target_span {
        Some(span) => span,
        None => return,
    };
    ctx.diagnostics.truncate(checkpoint);
    ctx.push(diagnostic_with_syntax_span(
        crate::checks::relation_chain::with_chain(
            Diagnostic::ts2322(source.name(), expected_type.name(), ctx.file_name.clone()),
            &crate::checks::relation_chain::chain_beneath_head(&source, expected_type),
        ),
        Some(span),
    ));
}

fn object_literal_against_union_members(
    union: &surge_ts_types::UnionType,
    properties: &[ParsedObjectProperty],
    expression: &ParsedExpression,
    expected_type: &Type,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    _expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<InferredExpression> {
    {
        let written: Vec<&str> = properties
            .iter()
            .filter(|property| !property.is_spread)
            .map(|property| property.name.as_str())
            .collect();
        let flattened_for_excess = flattened_union_members(union);
        // tsc discriminates the target by the literal's own unit-typed
        // properties first (`discriminateTypeByDiscriminableItems`): with
        // `kind: "one"` written, the literal is checked against that member
        // alone, so a property only the *other* member declares is excess.
        // Only the excess-property decision is taken from the discriminated
        // member: everything else about the literal is still related to the
        // union, where a missing property stays the union's TS2322.
        // Peeling every member is the expensive part, so it is done once and
        // only for a literal that could have an excess property at all.
        let closed_members = (!written.is_empty()
            && !properties.iter().any(|property| property.is_spread))
        .then(|| closed_object_members(&flattened_for_excess))
        .flatten();
        let excess = closed_members.as_deref().and_then(|objects| {
            match discriminated_object_member(objects, properties) {
                Some(member) => properties
                    .iter()
                    .find(|property| !member.contains_property(&property.name))
                    .map(|property| (property, Type::Object(member.clone()).name())),
                // Undiscriminated, a property is excess when no member knows it
                // (`isKnownProperty` over the union), and tsc names the union.
                None => union_excess_property(objects, properties),
            }
        });
        if !ctx.relating_unfresh_return
            && let Some((property, union_name)) = excess
        {
            let diagnostic =
                Diagnostic::ts2353(written_object_property_name(property), &union_name, ctx.file_name.clone());
            ctx.push(diagnostic_with_syntax_span(
                diagnostic,
                choose_span(
                    property.name_span,
                    choose_span(property.span, fallback_span),
                ),
            ));
            return Some(InferredExpression::Unknown);
        }
        // Only an unambiguous match is used: one member must declare every
        // written property and cover strictly more of them than any other. A
        // discriminated union (every member carries `code`, `message`, …) ties
        // and stays context-free, since picking a member there needs discriminant
        // matching and guessing wrong reports the literal against the wrong one.
        let flattened = flattened_union_members(union);
        // A function member answers `name` and `length` from `Function`, but no
        // object literal is a function (React's `S | ((prev: S) => S)`).
        let mut matching = flattened.iter().filter(|member| {
            !matches!(member.peeled(), Type::Function(_))
                && written
                    .iter()
                    .any(|name| member.get_property_access_type(name).is_some())
        });
        // The sole match must also *declare* one of the written properties: a
        // string index signature answers every name, so a member that only
        // index-accesses them models nothing about the literal and typing it
        // against that member reports the member's own required properties as
        // missing.
        let unambiguous = match (matching.next(), matching.next()) {
            (Some(member), None)
                if !written.is_empty()
                    && written
                        .iter()
                        .any(|name| member_declares_property(member, name)) =>
            {
                Some(member)
            }
            _ => None,
        };
        // The literal is checked against one member, but it is the *union* it
        // fails: tsc keeps the outer assignability code for a missing property
        // there (the TS2741 text is only its elaboration), as it does for an
        // intersection. A union of at most three that is one object plus
        // `undefined` or `null` is that object (`isRelatedTo` strips the
        // nullables), and keeps TS2741.
        let union_target = (flattened.len() > 3
            || flattened
                .iter()
                .filter(|member| !matches!(member, Type::Undefined | Type::Null | Type::Void))
                .count()
                > 1)
        .then(|| expected_type.clone());
        if let Some(member) = unambiguous {
            let checkpoint = ctx.diagnostics.len();
            ctx.union_literal_target = union_target.clone();
            let result = evaluate_expression_with_expected_type_anchored(
                expression,
                fallback_span,
                target_span,
                Some(member),
                _expected_diagnostic,
                symbols,
                ctx,
            );
            ctx.union_literal_target = None;
            rehead_unelaborated_union_properties(
                checkpoint,
                properties,
                &flattened,
                expression,
                expected_type,
                target_span,
                _expected_diagnostic,
                symbols,
                ctx,
            );
            rehead_missing_members_against_union(
                checkpoint,
                expression,
                expected_type,
                _expected_diagnostic,
                symbols,
                ctx,
            );
            return Some(result);
        }

        // Several members declare the written properties, and the one the literal
        // belongs to may only be visible *below* the top level:
        // `{ value: A[] } | { value: B[] }` ties on names, and what decides it is
        // a literal inside the nested array. Typing the literal once against the
        // per-property union — which is the contextual type tsc uses here — gives
        // each nested literal the context it needs, and the result then picks its
        // member. Two passes, not one per candidate.
        if members_are_all_objects(&flattened)
            && let Some(member) = union_member_for_object_literal(
                &flattened,
                expression,
                &written,
                fallback_span,
                target_span,
                _expected_diagnostic,
                symbols,
                ctx,
            )
        {
            let checkpoint = ctx.diagnostics.len();
            ctx.union_literal_target = union_target.clone();
            let result = evaluate_expression_with_expected_type_anchored(
                expression,
                fallback_span,
                target_span,
                Some(&member),
                _expected_diagnostic,
                symbols,
                ctx,
            );
            ctx.union_literal_target = None;
            rehead_unelaborated_union_properties(
                checkpoint,
                properties,
                &flattened,
                expression,
                expected_type,
                target_span,
                _expected_diagnostic,
                symbols,
                ctx,
            );
            rehead_missing_members_against_union(
                checkpoint,
                expression,
                expected_type,
                _expected_diagnostic,
                symbols,
                ctx,
            );
            return Some(result);
        }

        // Several members declare the written properties (an overload group's
        // merged parameter, where every member extends the same base). tsc types
        // the literal by the union of each property across those members, so
        // that is tried first: each property under the union of what the
        // candidates declare for it, accepted when the result fits the union.
        // Failing that, surge cannot pick a member and evaluates context-free —
        // but an implicit-any report there would describe that gap, not the
        // source.
        if members_are_all_objects(&flattened) {
            if let Some(result) = object_literal_under_property_unions(
                expected_type,
                &flattened,
                properties,
                fallback_span,
                target_span,
                _expected_diagnostic,
                symbols,
                ctx,
            ) {
                return Some(result);
            }
            ctx.degraded_expected_type_depth += 1;
            let result = evaluate_expression(expression, fallback_span, symbols, ctx);
            ctx.degraded_expected_type_depth -= 1;
            return Some(result);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn evaluate_expression_with_expected_type_rest(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_type: &Type,
    _expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {

    // Contextual typing through a union: when the expected type is a union whose
    // only non-nullish member is a single concrete type (e.g. `{ ... } | null`,
    // mapped here to `{ ... } | undefined`), use that member as the contextual
    // type so an object/array literal's property values are evaluated with the
    // expected element types rather than context-free (which would widen
    // member-access values like `res.status` toward `unknown`).
    if let Type::Union(union) = expected_type {
        let mut non_nullish = union
            .types()
            .iter()
            .filter(|member| !matches!(member, Type::Undefined | Type::Void));
        if let (Some(member), None) = (non_nullish.next(), non_nullish.next()) {
            return evaluate_expression_with_expected_type_anchored(
                expression,
                fallback_span,
                target_span,
                Some(member),
                _expected_diagnostic,
                symbols,
                ctx,
            );
        }
    }

    evaluate_expression(expression, fallback_span, symbols, ctx)
}

/// tsc's `checkTemplateExpression` under a template-literal contextual type: a
/// template written where a string literal or template literal type is
/// expected is typed by its own pattern rather than as `string`, so
/// `` `abc${s}` `` satisfies `` `abc${string}` ``. A placeholder that is not
/// `string | number | boolean | bigint | null | undefined` reads as `string`.
fn contextual_template_literal_type(
    expression: &ParsedExpression,
    expected_type: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // A const context types the template by pattern whatever is expected.
    let (expression, const_context) = match expression {
        ParsedExpression::ConstAssertion { expression, .. } => (expression.as_ref(), true),
        other => (other, false),
    };
    let ParsedExpression::TemplateLiteral {
        expressions,
        quasis,
        ..
    } = expression
    else {
        return None;
    };
    if expressions.is_empty() || quasis.len() != expressions.len() + 1 {
        return None;
    }
    if const_context {
        return crate::infer::expression::template_expression_pattern_type(
            expressions,
            quasis,
            symbols,
            ctx,
        );
    }
    // `isTemplateLiteralContextualType`: a literal or template target, or a
    // type variable whose base constraint may be a string.
    let is_template_context = |ty: &Type| {
        matches!(ty, Type::StringLiteral(_))
            || surge_ts_types::is_template_literal_type(ty)
            || surge_ts_types::type_variable::template_literal_variable_parts(ty).is_some()
            || ty.is_type_variable()
                && match surge_ts_types::type_variable::base_constraint_or_type(ty) {
                    Type::Union(union) => union.types().iter().any(is_string_like_constraint),
                    other => is_string_like_constraint(&other),
                }
    };
    let in_template_context = match expected_type {
        Type::Union(union) => flattened_union_members(union).iter().any(is_template_context),
        other => is_template_context(other),
    };
    if !in_template_context {
        return None;
    }
    crate::infer::expression::template_expression_pattern_type(expressions, quasis, symbols, ctx)
}

fn is_string_like_constraint(ty: &Type) -> bool {
    matches!(ty, Type::String | Type::StringLiteral(_))
        || surge_ts_types::is_template_literal_type(ty)
        || surge_ts_types::string_mapping_parts(ty).is_some()
}

/// Whether `member` declares `name` as a real property.
///
/// A string index signature (and the `Object.prototype` member fallback) answers
/// every name, so `get_property_access_type` cannot tell the member that models
/// the written property from one that merely permits it. Picking the latter
/// types the literal against the wrong shape — e.g. an object literal against
/// `ReadableStream | ... | Record<string, any>` was reported as missing
/// `ReadableStream`'s members once a `.d.ts` merge gave that interface a
/// degraded index signature.
/// The union member an object literal belongs to, when several members declare
/// everything it writes. Without it the literal is evaluated context-free, its
/// *nested* literals widen, and every member then rejects it.
///
/// Two signals, cheapest first. A property written as a primitive literal has its
/// type without any evaluation, and in a discriminated union it alone decides.
/// Only a literal with no such property — `{ value: [ … ] }`, whose discriminator
/// lives inside the array — pays for one real evaluation of that one property
/// against the union of what the candidates declare for it.
///
/// Trying each candidate instead was measured and rejected: 5 false positives on
/// tRPC and tanstack-query never finishing.
/// The union's members with any member that is itself a union spliced in.
///
/// A union reached through a reference stays nested: `type MaybeArray<T> = T |
/// readonly T[]` used as a type *argument*
/// (`LazyOrAsync<MaybeArray<UserConfig>>`) contributes one lazy reference, not
/// its two constituents, where the same alias written directly is peeled at the
/// top and does contribute them. tsc's `getUnionType` flattens transitively, so
/// the nested member has to be read through here — otherwise no candidate
/// declares the literal's properties, the literal is typed context-free, and a
/// nested array literal's elements widen away from the literal union they were
/// meant to match.
fn flattened_union_members(union: &surge_ts_types::UnionType) -> Vec<Type> {
    let members = union.types();
    if !members
        .iter()
        .any(|member| matches!(member, Type::Reference(_)))
    {
        return members.to_vec();
    }
    let mut flattened = Vec::with_capacity(members.len());
    for member in members {
        match member.peeled() {
            Type::Union(nested) => flattened.extend(nested.types().iter().cloned()),
            _ => flattened.push(member.clone()),
        }
    }
    flattened
}

fn union_member_for_object_literal(
    members: &[Type],
    expression: &ParsedExpression,
    written: &[&str],
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if written.is_empty() {
        return None;
    }
    let candidates: Vec<&Type> = members
        .iter()
        .filter(|member| {
            written
                .iter()
                .all(|name| member_declares_property(member, name))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    // Typing a literal against a *wide* member is the expensive half of this, not
    // the probe: with no cap at all, and even with every probe removed, the
    // tanstack-query aggregate goes from 4 s to not finishing in five minutes on
    // its query-option unions. At 20 own properties it is 4 s again, and 40 is
    // already too high. The cap is a cost bound, so it is deliberately about the
    // candidate's size rather than the union's arity.
    const MAX_CANDIDATE_PROPERTIES: usize = 20;
    if candidates.iter().any(|member| {
        matches!(member.peeled(), Type::Object(object) if object.properties.len() > MAX_CANDIDATE_PROPERTIES)
    }) {
        return None;
    }

    let mut narrowed: Vec<&Type> = candidates.clone();
    // One member declaring *every* written property, where several declare some,
    // is already the decision — that is the same evidence the single-match check
    // above uses, read the strict way round. Unless the literal fits another
    // member as it stands: tsc keeps the union as the contextual type when no
    // discriminant picks a constituent, and `{ a: '', b: '' }` is a `Bar` even
    // though only `Foo` declares both names.
    let mut decided = candidates.len() == 1 && members.len() > 1;
    // A written discriminant does pick the constituent, whatever else fits.
    let writes_discriminant = properties_of(expression).iter().any(|property| {
        !property.is_spread
            && written_literal_value(&property.value).is_some()
            && members.iter().any(|member| {
                matches!(
                    member.get_property_access_type(property.name.as_str()),
                    Some(Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_))
                )
            })
    });
    if decided && !writes_discriminant {
        let diagnostics_before = ctx.diagnostics.len();
        let context_free = crate::infer::infer_expression(expression, symbols, ctx);
        ctx.truncate_diagnostics(diagnostics_before);
        if let InferredExpression::Known(context_free) = context_free
            && !context_free.is_unknown()
            && !crate::checks::function::type_contains_unknown(&context_free)
            && members.iter().any(|member| {
                !std::ptr::eq(member, candidates[0]) && is_assignable_to(&context_free, member)
            })
        {
            return None;
        }
    }

    // Discriminants first, and for free: a property written as a primitive
    // literal has its type without any evaluation, and in a discriminated union
    // it alone decides. Only if that leaves the set ambiguous is anything typed.
    for property in properties_of(expression) {
        if property.is_spread {
            continue;
        }
        let Some(value) = written_literal_value(&property.value) else {
            continue;
        };
        narrowed = match narrow_by_property(&narrowed, property.name.as_str(), &value) {
            Some(next) => {
                decided = true;
                next
            }
            None => narrowed,
        };
        if narrowed.len() == 1 {
            return Some(narrowed[0].clone());
        }
    }

    // No discriminant settled it. One property may still, but typing it costs a
    // real evaluation, so this is bounded to the single-property literal — the
    // `{ value: [ … ] }` shape whose discriminator lives inside a nested array —
    // and cannot nest.
    const MAX_PROBE_CANDIDATES: usize = 4;
    if !decided
        && written.len() == 1
        && narrowed.len() <= MAX_PROBE_CANDIDATES
        && ctx.union_member_probe_depth == 0
        && let Some(property) = properties_of(expression)
            .iter()
            .find(|property| property.name.as_str() == written[0])
    {
        let per_property: Vec<Type> = narrowed
            .iter()
            .filter_map(|member| member.get_property_access_type(written[0]))
            .collect();
        if per_property.len() == narrowed.len() {
            let diagnostics_before = ctx.diagnostics().len();
            ctx.union_member_probe_depth += 1;
            let evaluated = evaluate_expression_with_expected_type_anchored(
                &property.value,
                property.value_span.or(fallback_span),
                target_span,
                Some(&surge_ts_types::union_type(per_property)),
                expected_diagnostic,
                symbols,
                ctx,
            );
            ctx.union_member_probe_depth -= 1;
            ctx.truncate_diagnostics(diagnostics_before);
            if let crate::infer::InferredExpression::Known(ty) = evaluated
                && !ty.is_unknown()
                && let Some(next) = narrow_by_property(&narrowed, written[0], &ty)
            {
                decided = true;
                narrowed = next;
            }
        }
    }

    if !decided {
        return None;
    }

    let mut accepting = narrowed.into_iter();
    match (accepting.next(), accepting.next()) {
        (Some(member), None) => Some(member.clone()),
        _ => None,
    }
}

/// An object literal typed the way tsc types it against a union none of whose
/// members can be singled out: every property is evaluated under the union of
/// what the candidate members declare for it, and the literal is accepted only
/// when the shape that produces fits the union. `None` leaves the existing
/// context-free path to report; nothing is emitted here.
#[allow(clippy::too_many_arguments)]
fn object_literal_under_property_unions(
    expected_type: &Type,
    members: &[Type],
    properties: &[ParsedObjectProperty],
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<InferredExpression> {
    if properties.is_empty() || properties.iter().any(|property| property.is_spread) {
        return None;
    }
    let candidates: Vec<&Type> = members
        .iter()
        .filter(|member| {
            properties
                .iter()
                .all(|property| member_declares_property(member, &property.name))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let diagnostics_before = ctx.diagnostics().len();
    let mut inferred = surge_ts_types::PropertyMap::default();
    for property in properties {
        let per_property: Vec<Type> = candidates
            .iter()
            .filter_map(|member| member.get_property_access_type(&property.name))
            .collect();
        let evaluated = evaluate_expression_with_expected_type_anchored(
            &property.value,
            property.value_span.or(fallback_span),
            target_span,
            Some(&surge_ts_types::union_type(per_property)),
            expected_diagnostic,
            symbols,
            ctx,
        );
        let InferredExpression::Known(ty) = evaluated else {
            ctx.truncate_diagnostics(diagnostics_before);
            return None;
        };
        if ty.is_unknown() {
            ctx.truncate_diagnostics(diagnostics_before);
            return None;
        }
        inferred.insert(
            property.name.as_str().into(),
            surge_ts_types::ObjectProperty::required(ty),
        );
    }
    let literal = Type::Object(alloc_object_type(inferred, None));
    if ctx.diagnostics().len() != diagnostics_before || !is_assignable_to(&literal, expected_type) {
        ctx.truncate_diagnostics(diagnostics_before);
        if candidates.len() == members.len()
            && let Some(result) = elaborate_union_literal_properties(&literal, &candidates, properties, symbols, ctx)
        {
            return Some(result);
        }
        return None;
    }
    Some(InferredExpression::Known(literal))
}

/// `elaborateObjectLiteral` against a union every member of which declares
/// every written property: a property whose value fits none of the members'
/// types for it (`getBestMatchIndexedAccessTypeOrUndefined`) is reported on
/// its name, and the literal is not reported again as a whole. Only plain
/// values: an object, array or function value elaborates further down.
fn elaborate_union_literal_properties(
    literal: &Type,
    candidates: &[&Type],
    properties: &[ParsedObjectProperty],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<InferredExpression> {
    let mut failing = Vec::new();
    for property in properties {
        let source = literal.get_property_access_type(&property.name)?;
        if source.is_unmodelled() || crate::checks::function::type_contains_degradation(&source) {
            return None;
        }
        let declared: Vec<Type> = candidates
            .iter()
            .map(|member| member.get_property_access_type(&property.name))
            .collect::<Option<Vec<_>>>()?;
        let target = surge_ts_types::union_type(declared);
        if is_assignable_to(&source, &target) {
            continue;
        }
        failing.push(match &property.value {
            ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. } => return None,
            ParsedExpression::ArrowFunction(arrow) => match arrow_elaboration(arrow, &source, &target, symbols, ctx)? {
                Some((returned, expected)) => (arrow.body_span?, returned, expected),
                None => (property.name_span?, source, target),
            },
            _ => (property.name_span?, source, target),
        });
    }
    if failing.is_empty() {
        return None;
    }
    for (span, source, target) in failing {
        let source_name = crate::checks::expr::source_display_name(&source, &target);
        ctx.push(diagnostic_with_syntax_span(
            crate::checks::relation_chain::with_chain(
                Diagnostic::ts2322(source_name, target.name(), ctx.file_name.clone()),
                &crate::checks::relation_chain::chain_beneath_head(&source, &target),
            ),
            Some(span),
        ));
    }
    Some(InferredExpression::Known(Type::Unknown))
}

/// tsc's `elaborateArrowFunction`: an expression-bodied arrow with no
/// annotated parameter whose return the target's signatures reject is
/// reported at its return expression (`Some` of the two returns), any other
/// at the property (`Some(None)`). `None` where surge cannot tell: the body
/// is judged by its own type, read only when it names no parameter.
fn arrow_elaboration(
    arrow: &surge_ts_syntax::ParsedArrowFunction,
    source: &Type,
    target: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Option<(Type, Type)>> {
    let surge_ts_syntax::ParsedArrowFunctionBody::Expression(body) = &arrow.body else {
        return Some(None);
    };
    if arrow.parameters.iter().any(|parameter| parameter.declared_type.is_some()) {
        return Some(None);
    }
    if !matches!(source.peeled(), Type::Function(_)) {
        return None;
    }
    let mut parameter_names = Vec::new();
    for parameter in &arrow.parameters {
        let surge_ts_syntax::ParsedBindingName::Identifier { name, .. } = &parameter.binding_name else {
            return None;
        };
        parameter_names.push(name.as_str());
    }
    let mut reads = crate::checks::function::ValueReads::default();
    crate::checks::function::collect_value_reads(body, false, &mut reads);
    if reads.eager.iter().chain(&reads.deferred).any(|read| parameter_names.contains(&read.as_str())) {
        return None;
    }
    let checkpoint = ctx.diagnostics().len();
    let returned = evaluate_expression(body, arrow.body_span, symbols, ctx);
    ctx.truncate_diagnostics(checkpoint);
    let InferredExpression::Known(returned) = returned else {
        return None;
    };
    if crate::checks::call::type_contains_unknown(&returned) {
        return None;
    }
    let members: Vec<Type> = match target.peeled() {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other.clone()],
    };
    let mut returns = Vec::with_capacity(members.len());
    for member in &members {
        match member.peeled() {
            Type::Function(signature) if signature.overloads().is_none() => returns.push(signature.return_type().clone()),
            _ => return None,
        }
    }
    let expected = surge_ts_types::union_type(returns);
    Some((!is_assignable_to(&returned, &expected)).then_some((returned, expected)))
}

/// A property written as a primitive literal, as the type that literal has. The
/// discriminant of a discriminated union is always one of these, and reading it
/// costs no evaluation.
fn written_literal_value(expression: &ParsedExpression) -> Option<Type> {
    match expression {
        ParsedExpression::StringLiteral(value) => Some(Type::StringLiteral(value.clone())),
        ParsedExpression::BooleanLiteral(value) => Some(Type::BooleanLiteral(*value)),
        ParsedExpression::NumberLiteral(value) => {
            Some(Type::NumberLiteral(surge_ts_types::NumberLiteralType {
                value: value.clone(),
            }))
        }
        _ => None,
    }
}

/// The candidates whose declared `name` accepts `value`, or `None` when that
/// rules out everything or nothing — neither is evidence.
fn narrow_by_property<'a>(
    candidates: &[&'a Type],
    name: &str,
    value: &Type,
) -> Option<Vec<&'a Type>> {
    let accepting: Vec<&Type> = candidates
        .iter()
        .copied()
        .filter(|member| {
            member
                .get_property_access_type(name)
                .is_some_and(|declared| surge_ts_types::is_assignable_to(value, &declared))
        })
        .collect();
    (!accepting.is_empty() && accepting.len() < candidates.len()).then_some(accepting)
}

/// An object literal's own properties, or an empty slice for anything else.
fn properties_of(expression: &ParsedExpression) -> &[ParsedObjectProperty] {
    match expression {
        ParsedExpression::ObjectLiteral { properties, .. } => properties,
        _ => &[],
    }
}

/// What the literal evaluates to once its own error has been reported. tsc
/// keeps checking the expression itself, so the value that travels outward is
/// the literal's own shape: substituting the constituent it was reported
/// against would relate differently wherever the value lands and report a
/// second, unrelated failure there.

/// Whether a type still mentions a type parameter anywhere surge can see,
/// which marks a contextual type whose call is mid-inference.
pub(crate) fn mentions_type_parameter(ty: &Type) -> bool {
    match ty {
        Type::TypeParameter(_) => true,
        Type::Reference(reference) => reference.arguments.iter().any(mentions_type_parameter),
        Type::Union(union) => union.types().iter().any(mentions_type_parameter),
        Type::Array(element) => mentions_type_parameter(element),
        Type::Tuple(elements) => elements.iter().any(mentions_type_parameter),
        Type::Object(object) => object
            .properties
            .iter()
            .any(|(_, property)| mentions_type_parameter(&property.ty)),
        _ => false,
    }
}

/// The first name the literal writes that none of the matched constituents
/// declares or answers through an index signature.
fn excess_property_written<'a>(
    members: &[Type],
    properties: &'a [ParsedObjectProperty],
) -> Option<&'a ParsedObjectProperty> {
    let answers = |name: &str| {
        members.iter().any(|member| match member.peeled() {
            Type::Object(object) => {
                object.string_index_type.is_some() || object.properties.contains_key(name)
            }
            _ => true,
        })
    };
    properties.iter().find(|property| {
        !property.is_spread
            && !answers(&property.name)
            && surge_ts_types::object_prototype_member_type(&property.name).is_none()
    })
}

/// Whether `member` requires a property the literal does not write. A spread
/// may supply any of them, so a literal that spreads is not judged here.
fn misses_required_property(member: &Type, properties: &[ParsedObjectProperty]) -> bool {
    if properties.iter().any(|property| property.is_spread) {
        return false;
    }
    let Type::Object(object) = member.peeled() else {
        return false;
    };
    object.properties.iter().any(|(name, property)| {
        !property.optional
            && !properties
                .iter()
                .any(|written| written.name == name.as_ref())
    })
}

#[allow(clippy::too_many_arguments)]
fn discriminant_replacement_report(
    union: &surge_ts_types::UnionType,
    properties: &[ParsedObjectProperty],
    expression: &ParsedExpression,
    expected_type: &Type,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<(Diagnostic, Option<SyntaxTextSpan>)> {
    let flattened = flattened_union_members(union);
    // While a call's type arguments are still open the constituents carry type
    // parameters, and naming one of those describes the inference state.
    if flattened.iter().any(mentions_type_parameter) {
        return None;
    }
    let matched = discriminant_matched_members(&flattened, properties)?;
    let target = if matched.len() == 1 {
        matched[0].clone()
    } else {
        surge_ts_types::union_type(matched.clone())
    };
    // An excess key takes precedence: tsc reports one error per literal, and
    // that key is the one it names.
    if let Some(excess) = excess_property_written(&matched, properties) {
        return Some((
            excess_property_diagnostic(excess, &target, &target.name(), ctx),
            choose_span(excess.name_span, choose_span(excess.span, fallback_span)),
        ));
    }
    if !matched
        .iter()
        .all(|member| misses_required_property(member, properties))
    {
        return None;
    }
    let before = ctx.diagnostics().len();
    let source = match crate::infer::infer_expression(expression, symbols, ctx) {
        InferredExpression::Known(source) => source,
        _ => return None,
    };
    let source_name = source.name();
    ctx.truncate_diagnostics_releasing_utility_keys(before);
    let target_name = expected_type.name();
    let chain = crate::checks::relation_chain::chain_beneath_head(&source, expected_type);
    let diagnostic = match expected_diagnostic {
        ExpectedTypeDiagnostic::TypeNotAssignable => {
            Diagnostic::ts2322(&source_name, &target_name, ctx.file_name.clone())
        }
        ExpectedTypeDiagnostic::ArgumentNotAssignable => {
            Diagnostic::ts2345(&source_name, &target_name, ctx.file_name.clone())
        }
        ExpectedTypeDiagnostic::SatisfiesNotAssignable => {
            Diagnostic::ts1360(&source_name, &target_name, ctx.file_name.clone())
        }
        ExpectedTypeDiagnostic::ContextOnly => return None,
    };
    let diagnostic = crate::checks::relation_chain::with_chain(diagnostic, &chain);
    Some((diagnostic, choose_span(target_span, fallback_span)))
}

/// tsc's `getMatchingUnionConstituentForObjectLiteral`: a literal that writes a
/// discriminant with a unit value belongs to the constituent whose own
/// discriminant admits it, whatever else it writes. Without this a
/// discriminated union ties on names and the literal is reported against an
/// arbitrary member — `{ tag: "T", a1: "extra" }` read as a bad `tag` rather
/// than an excess `a1`.
fn discriminant_matched_members(
    members: &[Type],
    properties: &[ParsedObjectProperty],
) -> Option<Vec<Type>> {
    let unit_property_type = |member: &Type, name: &str| -> Option<Type> {
        let Type::Object(object) = member.peeled() else {
            return None;
        };
        let property = object.properties.get(name)?;
        let all_unit = match &property.ty {
            Type::Union(union) => union.types().iter().all(is_unit_type),
            other => is_unit_type(other),
        };
        all_unit.then(|| property.ty.clone())
    };

    for property in properties.iter().filter(|property| !property.is_spread) {
        let Some(written) = written_unit_type(&property.value) else {
            continue;
        };
        let declared: Vec<Option<Type>> = members
            .iter()
            .map(|member| unit_property_type(member, &property.name))
            .collect();
        if declared.iter().any(Option::is_none) {
            continue;
        }
        let admitting: Vec<Type> = members
            .iter()
            .zip(&declared)
            .filter(|(_, declared)| {
                declared
                    .as_ref()
                    .is_some_and(|declared| unit_type_admitted(&written, declared))
            })
            .map(|(member, _)| member.clone())
            .collect();
        if !admitting.is_empty() && admitting.len() < members.len() {
            return Some(admitting);
        }
    }
    None
}

/// The unit type a written property spells out, read straight from the syntax:
/// evaluating it here would run the inference engine inside a *selection*, and
/// what that leaves in the caches outlives the answer.
fn written_unit_type(value: &ParsedExpression) -> Option<Type> {
    match value {
        ParsedExpression::StringLiteral(text) => Some(Type::StringLiteral(text.clone())),
        ParsedExpression::NumberLiteral(text) => Some(Type::NumberLiteral(
            surge_ts_types::NumberLiteralType {
                value: text.clone(),
            },
        )),
        ParsedExpression::BooleanLiteral(value) => Some(Type::BooleanLiteral(*value)),
        _ => None,
    }
}

fn is_unit_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
            | Type::Undefined
            | Type::Null
    )
}

fn unit_type_admitted(written: &Type, declared: &Type) -> bool {
    match declared {
        Type::Union(union) => union.types().iter().any(|member| member == written),
        other => other == written,
    }
}

/// The object members of a union target, when every member that could type an
/// object literal is a closed, fully modelled object: `undefined`/`null` and
/// primitives are skipped, and anything else — `any`, an index signature, an
/// open or empty object, a member surge could not resolve — knows every
/// property, so the union is no excess-property target at all.
fn closed_object_members(members: &[Type]) -> Option<Vec<surge_ts_types::ObjectType>> {
    let mut objects = Vec::new();
    for member in members {
        match member.peeled() {
            Type::Undefined
            | Type::Void
            | Type::String
            | Type::Number
            | Type::Boolean
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_) => {}
            Type::Object(object)
                if !object.properties.is_empty()
                    && !object.allows_string_index_access()
                    && object.call_signature().is_none()
                    && object.construct_signature().is_none()
                    // Shallow on purpose, like `expected_member_is_degraded`:
                    // this runs for every object literal against a union, and
                    // the deep walk resolves every lazy reference in every
                    // member — measured at 93% of a tanstack run that no longer
                    // finished.
                    && !object
                        .properties
                        .values()
                        .any(|property| expected_member_is_degraded(&property.ty)) =>
            {
                objects.push(object);
            }
            _ => return None,
        }
    }
    (objects.len() >= 2).then_some(objects)
}

fn discriminated_object_member<'a>(
    objects: &'a [surge_ts_types::ObjectType],
    properties: &[ParsedObjectProperty],
) -> Option<&'a surge_ts_types::ObjectType> {
    let is_unit = |ty: &Type| {
        matches!(
            ty,
            Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_)
        )
    };
    let mut candidates: Vec<&surge_ts_types::ObjectType> = objects.iter().collect();
    let mut discriminated = false;
    for property in properties {
        let written = match &property.value {
            ParsedExpression::StringLiteral(value) => Type::StringLiteral(value.clone()),
            ParsedExpression::BooleanLiteral(value) => Type::BooleanLiteral(*value),
            ParsedExpression::NumberLiteral(value) => {
                Type::NumberLiteral(surge_ts_types::NumberLiteralType {
                    value: value.clone(),
                })
            }
            ParsedExpression::NullLiteral if surge_ts_types::strict_null_checks() => Type::Null,
            _ => continue,
        };
        let declared: Option<Vec<Type>> = objects
            .iter()
            .map(|object| {
                object
                    .properties
                    .get(property.name.as_str())
                    .filter(|declared| !declared.optional)
                    .map(|declared| declared.ty.peeled())
            })
            .collect();
        let Some(declared) = declared else {
            continue;
        };
        // A required property every member declares with a unit type; or, as
        // tsc's `CheckFlags.Discriminant`, one some member declares `null` and
        // another a different type it rejects.
        let all_units = declared.iter().all(is_unit);
        let null_discriminant = matches!(written, Type::Null)
            && declared.iter().any(|ty| matches!(ty, Type::Null))
            && declared.iter().any(|ty| !matches!(ty, Type::Null))
            && declared
                .iter()
                .all(|ty| matches!(ty, Type::Null) || (is_unit(ty) || matches!(ty, Type::String | Type::Number | Type::Boolean)));
        if !all_units && !null_discriminant {
            continue;
        }
        discriminated = true;
        candidates.retain(|object| {
            object
                .properties
                .get(property.name.as_str())
                .is_some_and(|declared| declared.ty.peeled() == written)
        });
    }
    match candidates.as_slice() {
        [only] if discriminated => Some(*only),
        _ => None,
    }
}

fn union_excess_property<'a>(
    objects: &[surge_ts_types::ObjectType],
    properties: &'a [ParsedObjectProperty],
) -> Option<(&'a ParsedObjectProperty, String)> {
    let property = properties.iter().find(|property| {
        !objects
            .iter()
            .any(|object| object.contains_property(&property.name))
    })?;
    // Rendered only once a property is known to be excess.
    let union_name =
        surge_ts_types::union_type(objects.iter().cloned().map(Type::Object).collect()).name();
    Some((property, union_name))
}

fn member_declares_property(member: &Type, name: &str) -> bool {
    matches!(member.peeled(), Type::Object(object) if object.properties.contains_key(name))
}

/// Whether every member of a union is an object type — the shape where a
/// written object literal is genuinely contextually typed by tsc even though
/// surge cannot pick a single member to check against.
fn members_are_all_objects(members: &[Type]) -> bool {
    let mut object_members = 0usize;
    for member in members {
        // An optional parameter contributes `undefined`; it is not a shape the
        // literal could be typed by, so it does not disqualify the union.
        if matches!(member, Type::Undefined | Type::Null | Type::Void) {
            continue;
        }
        if !matches!(member.peeled(), Type::Object(_)) {
            return false;
        }
        object_members += 1;
    }
    object_members >= 2
}

/// The `??` result rule, mirroring the context-free arm in
/// `checks::expr::evaluate`: the left operand contributes only its non-nullish
/// part, and either side failing to resolve degrades the whole expression.
fn join_nullish_coalescing(
    left_result: InferredExpression,
    right_result: InferredExpression,
) -> InferredExpression {
    match (left_result, right_result) {
        (InferredExpression::Known(left_type), InferredExpression::Known(right_type)) => {
            InferredExpression::Known(crate::infer::expression::nullish_coalescing_result(
                left_type, right_type,
            ))
        }
        _ => InferredExpression::Unknown,
    }
}

/// A union of plain written function types (`((s: string, w: boolean) =>
/// void) | ((s: string, w: number) => string)`), or of types with one call
/// signature each, which no overload merge produces, whose parameters differ:
/// signatures that are not identical give no contextual signature.
fn written_function_union(members: &[&Type]) -> bool {
    let functions: Vec<surge_ts_types::FunctionType> = members
        .iter()
        .filter_map(|member| match contextual_call_signature(member) {
            Some(function)
                if function.type_parameter_head().is_none()
                    && function.overloads().is_none()
                    && function.this_type().is_none()
                    && !function
                        .parameters()
                        .iter()
                        .any(crate::checks::function::type_contains_degradation) =>
            {
                Some(function)
            }
            _ => None,
        })
        .collect();
    functions.len() == members.len()
        && functions
            .iter()
            .any(|function| function.parameters() != functions[0].parameters())
}

/// Whether every callable member takes the same number of parameters.
fn callable_members_share_arity(members: &[&Type]) -> bool {
    let mut arity = None;
    for member in members {
        let Some(signature) = contextual_call_signature(member) else {
            return false;
        };
        let count = signature.parameters().len();
        match arity {
            None => arity = Some(count),
            Some(existing) if existing == count => {}
            Some(_) => return false,
        }
    }
    arity.is_some()
}

/// checker.go `getContextualCallSignature` over several call signatures:
/// those taking at least the function's required parameters
/// (`isAritySmaller`), one alone or combined (`getIntersectedSignatures`,
/// under `noImplicitAny` only). `None` keeps surge's folded signature.
fn intersected_call_signature(
    group: &surge_ts_types::FunctionType,
    arrow: &surge_ts_syntax::ParsedArrowFunction,
    ctx: &CheckerContext,
) -> Option<surge_ts_types::FunctionType> {
    let required = arrow
        .parameters
        .iter()
        .take_while(|parameter| parameter.initializer.is_none() && !parameter.optional && !parameter.rest)
        .count();
    let mut members = Vec::new();
    group.push_overload_members(&mut members);
    let applicable: Vec<_> = members
        .into_iter()
        .filter(|member| member.is_variadic() || member.parameters().len() >= required)
        .collect();
    match applicable.as_slice() {
        [single] => Some(single.clone()),
        [] => None,
        _ if ctx.options.no_implicit_any => super::call::intersected_contextual_signature(&applicable),
        _ => None,
    }
}

/// The call signature a union member contributes to contextual typing.
fn contextual_call_signature(ty: &Type) -> Option<surge_ts_types::FunctionType> {
    let peeled = ty.peeled();
    match peeled {
        Type::Function(function) => Some(function),
        Type::Object(object) => object.call_signature().cloned(),
        _ => None,
    }
}

fn is_contextual_callable(ty: &Type) -> bool {
    let peeled;
    let ty = match ty {
        Type::Reference(reference) => {
            peeled = reference.resolve().peeled();
            &peeled
        }
        other => other,
    };
    match ty {
        Type::Function(_) => true,
        Type::Object(object) => object.call_signature().is_some(),
        _ => false,
    }
}

fn evaluate_array_literal_in_context(
    elements: &[surge_ts_syntax::ParsedArrayElement],
    expected_type: &Type,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let mut element_types = Vec::with_capacity(elements.len());
    for (index, element) in elements.iter().enumerate() {
        let slot = match expected_type {
            Type::Array(element_type) => Some(element_type.as_ref()),
            Type::Tuple(slots) => slots.get(index),
            Type::OpenTuple(open) => open.leading.get(index).or(Some(open.rest.as_ref())),
            _ => None,
        };
        let inferred = match slot {
            Some(slot) => evaluate_expression_with_expected_type(
                &element.expression,
                element.span,
                Some(slot),
                ExpectedTypeDiagnostic::ContextOnly,
                symbols,
                ctx,
            ),
            None => crate::checks::expr::evaluate_expression(
                &element.expression,
                element.span.or(fallback_span),
                symbols,
                ctx,
            ),
        };
        match inferred {
            InferredExpression::Known(ty) => element_types.push(ty),
            _ => return InferredExpression::Unknown,
        }
    }
    InferredExpression::Known(match expected_type {
        Type::Array(_) => Type::Array(Box::new(surge_ts_types::union_type(element_types))),
        // Every element of the literal is present, however much `undefined`
        // its contextual slot admits (`[""]` against `[string | undefined]`).
        _ if elements.iter().all(|element| !element.spread) => {
            surge_ts_types::written_tuple_type(element_types, elements.len())
        }
        _ => Type::Tuple(element_types),
    })
}

fn evaluate_array_literal_with_expected_type(
    elements: &[surge_ts_syntax::ParsedArrayElement],
    expected_element_type: &Type,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let mut reported_element = false;
    let mut omitted_mismatch = false;
    let mut element_types = Vec::with_capacity(elements.len());
    for element in elements {
        let diagnostics_before = ctx.diagnostics().len();
        // `[...xs]` writes the *elements* of `xs` into the literal, so the
        // expected element type belongs to what `xs` yields, not to `xs`. The
        // operand is evaluated without it: handing an array the element type as
        // its context reported every spread as a false `TS2322`.
        let inferred_element = if element.spread {
            crate::infer::infer_expression(&element.expression, symbols, ctx)
        } else {
            evaluate_expression_with_expected_type(
                &element.expression,
                element.span,
                Some(expected_element_type),
                ExpectedTypeDiagnostic::TypeNotAssignable,
                symbols,
                ctx,
            )
        };

        match inferred_element {
            InferredExpression::Known(actual_type) => {
                let actual_type = if element.spread {
                    crate::checks::function::for_of_element_type(&actual_type)
                } else {
                    actual_type
                };
                if actual_type.is_unknown()
                    || crate::checks::call::is_open_instantiation(&actual_type)
                {
                    continue;
                }
                element_types.push(actual_type.clone());

                if element.omitted && !is_assignable_to(&actual_type, expected_element_type) {
                    omitted_mismatch = true;
                    continue;
                }
                if !is_assignable_to(&actual_type, expected_element_type) {
                    let actual_type =
                        crate::checks::expr::widen_type_for_contextual_type(&actual_type, expected_element_type);
                    let reported_target = crate::checks::expr::reported_relation_target(
                        &actual_type,
                        expected_element_type,
                    );
                    let actual_type_name =
                        crate::checks::expr::source_display_name(&actual_type, &reported_target);
                    let expected_type_name = reported_target.name();
                    let diagnostic = crate::checks::expr::type_not_assignable_diagnostic(
                        &actual_type,
                        &reported_target,
                        &actual_type_name,
                        &expected_type_name,
                        ctx.file_name.clone(),
                    );

                    ctx.push(diagnostic_with_syntax_span(
                        diagnostic,
                        choose_span(element.span, fallback_span),
                    ));
                    // `elaborateElementwise` reports every failing element.
                    reported_element = true;
                    continue;
                }
            }
            // An element whose own check already reported (an excess member)
            // leaves the rest to `elaborateElementwise`.
            InferredExpression::Unknown if !element.spread && ctx.diagnostics().len() > diagnostics_before => {
                reported_element = true;
            }
            InferredExpression::UnresolvedIdentifier { .. }
            | InferredExpression::MissingProperty { .. }
            | InferredExpression::Unknown => {
                return InferredExpression::Unknown;
            }
        }
    }
    if reported_element {
        return InferredExpression::Unknown;
    }
    // A mismatch only an omitted element carries is the whole literal's: its
    // own type goes back for the caller to report where it anchors the value.
    if omitted_mismatch {
        return InferredExpression::Known(Type::Array(Box::new(surge_ts_types::union_type(
            element_types,
        ))));
    }

    // `checkArrayLiteral`: an empty literal is `never[]` whatever its context.
    if elements.is_empty() && ctx.options.strict_null_checks {
        return InferredExpression::Known(Type::Array(Box::new(Type::Never)));
    }
    InferredExpression::Known(Type::Array(Box::new(with_type_copy_reason(
        TypeCopyReason::ExpectedType,
        || expected_element_type.clone(),
    ))))
}

fn mutable_sequence_shape(member: &Type) -> Type {
    // `ReadonlyArray<T>` spelled as a lib reference resolves to the readonly
    // wrapper, whose own resolution is the mutable shape; `peeled` follows
    // the whole chain.
    match member.peeled() {
        sequence @ (Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_)) => sequence,
        _ => member.clone(),
    }
}

/// The one array-or-tuple member of a union target an array literal can be: a
/// tuple of the same arity whose slots all accept the written elements, or an
/// array whose element type does. Element types are read *unwidened* — the
/// literal `'b'` is what tells `B[]` from `A[]`, and `['-', 2]` from
/// `['++', number]`. `None` when the literal is ambiguous or fits nothing, so
/// the caller keeps its context-free behavior.
fn sole_matching_sequence_member(
    members: &[Type],
    elements: &[surge_ts_syntax::ParsedArrayElement],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    // A spread element's arity is unknown, so no tuple member can be matched by
    // count and no slot lines up with a written position.
    let has_spread = elements.iter().any(|element| element.spread);
    let candidates: Vec<&Type> = members
        .iter()
        .filter(|member| match member {
            Type::Tuple(slots) => !has_spread && slots.len() == elements.len(),
            Type::Array(_) => true,
            // A rest slot takes any count from the fixed parts on; before a
            // spread only the leading run is written out.
            Type::OpenTuple(open) if has_spread => {
                elements.iter().take_while(|element| !element.spread).count() >= open.leading.len()
            }
            Type::OpenTuple(open) => elements.len() >= open.leading.len() + open.trailing.len(),
            _ => false,
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    if has_spread {
        return match candidates.as_slice() {
            [member @ Type::OpenTuple(_)] => Some((*member).clone()),
            _ => None,
        };
    }
    // `[]` writes nothing to tell members apart by, so it is the empty tuple
    // when the union declares one and otherwise the lone array member;
    // evaluated context-free it would be `any[]`, which fits no tuple.
    if elements.is_empty() {
        let mut empty_tuples = candidates
            .iter()
            .filter(|member| matches!(member, Type::Tuple(_)));
        return match (empty_tuples.next(), empty_tuples.next()) {
            (Some(member), None) => Some((*member).clone()),
            (None, _) if candidates.len() == 1 => Some(candidates[0].clone()),
            _ => None,
        };
    }

    let diagnostics_before = ctx.diagnostics().len();
    let mut written = Vec::with_capacity(elements.len());
    for element in elements {
        match crate::infer::infer_expression(&element.expression, symbols, ctx) {
            crate::infer::InferredExpression::Known(ty) if !ty.is_unknown() => written.push(ty),
            _ => {
                ctx.truncate_diagnostics(diagnostics_before);
                return None;
            }
        }
    }
    ctx.truncate_diagnostics(diagnostics_before);

    let mut fitting = candidates.iter().filter(|member| match member {
        Type::Tuple(slots) => slots
            .iter()
            .zip(written.iter())
            .all(|(slot, value)| surge_ts_types::is_assignable_to(value, slot)),
        Type::Array(element) => written
            .iter()
            .all(|value| surge_ts_types::is_assignable_to(value, element)),
        Type::OpenTuple(open) => written.iter().enumerate().all(|(index, value)| {
            let from_end = written.len() - index;
            let slot = if index < open.leading.len() {
                &open.leading[index]
            } else if from_end <= open.trailing.len() {
                &open.trailing[open.trailing.len() - from_end]
            } else {
                open.rest.as_ref()
            };
            surge_ts_types::is_assignable_to(value, slot)
        }),
        _ => false,
    });
    match (fitting.next(), fitting.next()) {
        (Some(member), None) => return Some((*member).clone()),
        // Several tuples of the literal's arity fit (`['line']` against
        // `['list'] | ['line'] | [string] | …`): tsc types each element by the
        // union of the candidates' slots, which keeps the literal a tuple of
        // literals that then fits the union target. Arrays among the fits
        // keep the literal ambiguous.
        (Some(first), Some(second)) => {
            let fitting_tuples: Vec<&[Type]> = std::iter::once(first)
                .chain(std::iter::once(second))
                .chain(fitting)
                .map(|member| match member {
                    Type::Tuple(slots) => Some(slots.as_slice()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            let slots = (0..elements.len())
                .map(|index| {
                    surge_ts_types::union_type(
                        fitting_tuples
                            .iter()
                            .map(|slots| slots[index].clone())
                            .collect(),
                    )
                })
                .collect();
            return Some(Type::Tuple(slots));
        }
        (None, _) => {}
    }

    // Nothing fit, which does not mean nothing belongs: an element that is
    // itself an object literal was inferred context-free above, so its own
    // nested literals widened and it matched no slot. Fall back to the test the
    // object-literal path uses one level up — the candidate whose slot *declares*
    // every property the element writes. That is what separates a datadog-shaped
    // `{ response_format, queries }` request from a `{ q }` one.
    let mut declaring = candidates.iter().filter(|member| {
        elements.iter().enumerate().all(|(index, element)| {
            let slot = match member {
                Type::Tuple(slots) => slots.get(index),
                Type::Array(element_type) => Some(element_type.as_ref()),
                _ => None,
            };
            match (slot, &element.expression) {
                (Some(slot), ParsedExpression::ObjectLiteral { properties, .. }) => properties
                    .iter()
                    .filter(|property| !property.is_spread)
                    .all(|property| member_declares_property(slot, property.name.as_str())),
                _ => false,
            }
        })
    });
    match (declaring.next(), declaring.next()) {
        (Some(member), None) => Some((*member).clone()),
        _ => None,
    }
}

/// Every element's own widened type, with an element that does not resolve
/// standing in as `any` — tsc's error type, which it renders the same way
/// (`Type '[string, number, any]' is not assignable to type '[string, number]'`).
/// The inference is a probe, so its diagnostics are discarded: the unresolved
/// element is reported by the pass that owns it, not by this message.
fn literal_element_types(
    elements: &[surge_ts_syntax::ParsedArrayElement],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Vec<Type> {
    let diagnostics_before = ctx.diagnostics().len();
    let mut element_types = Vec::with_capacity(elements.len());
    for element in elements {
        let element_type = match crate::infer::infer_expression(&element.expression, symbols, ctx) {
            crate::infer::InferredExpression::Known(ty) if !ty.is_unknown() => {
                crate::checks::expr::widen_type(&ty)
            }
            _ => Type::Any,
        };
        element_types.push(element_type);
    }
    ctx.truncate_diagnostics(diagnostics_before);
    element_types
}

/// The slot of a tuple-like target at `index`, counted from the front. Only
/// meaningful before the literal's first variable-length spread.
fn tuple_like_slot(expected_type: &Type, index: usize) -> Option<Type> {
    match expected_type {
        Type::Tuple(slots) => slots.get(index).cloned(),
        // `getContextualTypeForElementExpression` maps over a union: each member
        // that has the slot contributes it.
        Type::Union(union) => {
            let slots: Vec<Type> = union
                .types()
                .iter()
                .filter_map(|member| tuple_like_slot(&mutable_sequence_shape(member), index))
                .collect();
            (!slots.is_empty()).then(|| surge_ts_types::union_type(slots))
        }
        Type::OpenTuple(open) => Some(open.leading.get(index).unwrap_or(open.rest.as_ref()).clone()),
        other => match other.peeled() {
            Type::Object(object) => object.get_property_access_type(&index.to_string()),
            _ => None,
        },
    }
}

/// tsc's `isTupleLikeType` for a target that is not itself an array or a
/// tuple: one declaring a property `0`, or an array-like one whose `length`
/// is a number literal. An array literal it contextually types is a tuple
/// (`checkArrayLiteral`'s `inTupleContext`).
fn is_tuple_like_object_target(expected_type: &Type) -> bool {
    let peeled = expected_type.peeled();
    let Type::Object(object) = &peeled else {
        return false;
    };
    if object.get_property("0").is_some_and(|property| !property.index_slot) {
        return true;
    }
    let literal_length = object.get_property("length").is_some_and(|property| {
        !property.index_slot
            && match &property.ty {
                Type::NumberLiteral(_) => true,
                Type::Union(union) => union.types().iter().all(|member| matches!(member, Type::NumberLiteral(_))),
                _ => false,
            }
    });
    literal_length && is_assignable_to(&peeled, &Type::Array(Box::new(Type::Any)))
}

/// An intersection kept as its operands with a deferred conditional type among
/// them (the intersection arm of `isOrHasGenericConditional`).
fn has_generic_conditional_operand(object: &surge_ts_types::ObjectType) -> bool {
    object.is_intersection
        && object.intersection_operands.as_deref().is_some_and(|operands| {
            operands.iter().any(|operand| match operand {
                Type::TypeParameter(parameter) => matches!(
                    surge_ts_types::type_variable::deferred_type(parameter),
                    Some(surge_ts_types::type_variable::DeferredType::Conditional(_))
                ),
                Type::Object(nested) => has_generic_conditional_operand(nested),
                _ => false,
            })
        })
}

/// An array literal with spread elements against a tuple-like target. The
/// literal's own tuple type is built the way tsc builds it and related to the
/// target as a whole; node positions no longer line up with slots, so a
/// failure is never elaborated onto an element. A spread surge cannot read as
/// an array or tuple leaves the literal unreported.
fn evaluate_spread_tuple_literal(
    elements: &[surge_ts_syntax::ParsedArrayElement],
    expected_type: &Type,
    elaborate_elements: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let checkpoint = ctx.diagnostics().len();
    let mut leading: Vec<Type> = Vec::new();
    let mut rest: Option<Type> = None;
    let mut trailing: Vec<Type> = Vec::new();
    let mut modelled = true;

    for element in elements {
        if element.spread {
            let operand = match evaluate_expression(&element.expression, element.span, symbols, ctx)
            {
                InferredExpression::Known(ty) => ty.peeled(),
                _ => {
                    modelled = false;
                    continue;
                }
            };
            match operand {
                Type::Tuple(members) if rest.is_none() => leading.extend(members),
                Type::Tuple(members) => trailing.extend(members),
                Type::Array(member) if rest.is_none() => rest = Some(*member),
                Type::OpenTuple(open) if rest.is_none() => {
                    leading.extend(open.leading);
                    rest = Some(*open.rest);
                    trailing.extend(open.trailing);
                }
                _ => modelled = false,
            }
            continue;
        }

        let slot = if rest.is_none() {
            tuple_like_slot(expected_type, leading.len())
        } else {
            None
        };
        let slot = slot.as_ref();
        let element_type = match evaluate_expression_with_expected_type(
            &element.expression,
            element.span,
            slot,
            ExpectedTypeDiagnostic::TypeNotAssignable,
            symbols,
            ctx,
        ) {
            InferredExpression::Known(ty) if !ty.is_unknown() => ty,
            InferredExpression::Known(_) => Type::Any,
            _ => {
                modelled = false;
                continue;
            }
        };
        // A literal that fits its slot stays as written; anything else is
        // named by its widened type, as tsc names it.
        let element_type = match slot {
            Some(slot) if is_assignable_to(&element_type, slot) => element_type,
            _ => crate::checks::expr::widen_type(&element_type),
        };
        if rest.is_none() {
            leading.push(element_type);
        } else {
            trailing.push(element_type);
        }
    }

    if !modelled {
        return InferredExpression::Unknown;
    }
    let literal_type = match rest {
        Some(rest) => Type::OpenTuple(surge_ts_types::OpenTupleType {
            leading,
            rest: Box::new(rest),
            trailing,
        }),
        None => Type::Tuple(leading),
    };
    if is_assignable_to(&literal_type, expected_type) {
        return InferredExpression::Known(expected_type.clone());
    }
    // Without a spread every element still has its slot, so an element the
    // target rejected was elaborated onto (`elaborateArrayLiteral`).
    if elaborate_elements
        && elements.iter().all(|element| !element.spread)
        && ctx.diagnostics().len() > checkpoint
    {
        return InferredExpression::Unknown;
    }

    // See `evaluate_open_tuple_literal_with_expected_type`: a nested literal's
    // own elaboration gives way to the whole value's report, anything else
    // raised inside an element stands on its own.
    let only_assignability = ctx.diagnostics()[checkpoint..].iter().all(|diagnostic| {
        matches!(
            diagnostic.code,
            surge_ts_diagnostics::DiagnosticCode::TypeScript(2322 | 2353 | 2739 | 2740 | 2741)
        )
    });
    if !only_assignability {
        return InferredExpression::Unknown;
    }
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    // The failing literal is named by its widened elements, and a literal that
    // is nothing but a spread array is that array.
    let widen_all = |types: Vec<Type>| -> Vec<Type> {
        types.iter().map(crate::checks::expr::widen_type).collect()
    };
    InferredExpression::Known(match literal_type {
        Type::OpenTuple(open) if open.fixed_len() == 0 => Type::Array(open.rest),
        Type::OpenTuple(open) => Type::OpenTuple(surge_ts_types::OpenTupleType {
            leading: widen_all(open.leading),
            rest: open.rest,
            trailing: widen_all(open.trailing),
        }),
        Type::Tuple(members) => Type::Tuple(widen_all(members)),
        other => other,
    })
}

/// An array literal against a tuple with a rest slot. tsc's
/// `elaborateArrayLiteral` reads the target's element type by index, which only
/// a *leading* fixed slot answers: a wrong element there is reported on the
/// element, as for a fixed tuple. A wrong element in the rest or a trailing
/// slot, and a literal shorter than the fixed slots, leave nothing to descend
/// into, so the literal's own tuple type goes back to the caller to report as
/// a whole.
fn evaluate_open_tuple_literal_with_expected_type(
    elements: &[surge_ts_syntax::ParsedArrayElement],
    open: &surge_ts_types::OpenTupleType,
    expected_type: &Type,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    if elements.len() < open.fixed_len() {
        return InferredExpression::Known(Type::Tuple(literal_element_types(
            elements, symbols, ctx,
        )));
    }

    let (leading, rest_and_trailing) = elements.split_at(open.leading.len());
    if !leading.is_empty()
        && !matches!(
            evaluate_tuple_literal_with_expected_type(
                leading,
                &open.leading,
                None,
                fallback_span,
                None,
                false,
                symbols,
                ctx,
            ),
            InferredExpression::Known(_)
        )
    {
        return InferredExpression::Unknown;
    }

    let rest_count = rest_and_trailing.len() - open.trailing.len();
    let slots = std::iter::repeat_n(open.rest.as_ref(), rest_count).chain(open.trailing.iter());
    let checkpoint = ctx.diagnostics().len();
    let mut fits = true;
    for (element, slot) in rest_and_trailing.iter().zip(slots) {
        match evaluate_expression_with_expected_type(
            &element.expression,
            element.span,
            Some(slot),
            ExpectedTypeDiagnostic::TypeNotAssignable,
            symbols,
            ctx,
        ) {
            InferredExpression::Known(actual_type) => {
                if !actual_type.is_unknown()
                    && !crate::checks::call::is_open_instantiation(&actual_type)
                {
                    fits &= is_assignable_to(&actual_type, slot);
                }
            }
            _ => fits = false,
        }
    }
    if fits {
        return InferredExpression::Known(expected_type.clone());
    }

    // A nested literal that failed its slot has already elaborated into itself
    // (`{ a: "s" }` onto `a`). tsc does not descend through a rest slot, so
    // those reports give way to the whole value's — but only those: anything
    // else raised inside an element is an error of its own and stays, and the
    // literal is then left unreported rather than typed from a broken element.
    let only_assignability = ctx.diagnostics()[checkpoint..].iter().all(|diagnostic| {
        matches!(
            diagnostic.code,
            surge_ts_diagnostics::DiagnosticCode::TypeScript(2322 | 2353 | 2739 | 2740 | 2741)
        )
    });
    if !only_assignability {
        return InferredExpression::Unknown;
    }
    let excess_property_span = ctx.diagnostics()[checkpoint..]
        .iter()
        .find(|diagnostic| {
            matches!(
                diagnostic.code,
                surge_ts_diagnostics::DiagnosticCode::TypeScript(2353)
            )
        })
        .and_then(|diagnostic| diagnostic.span);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    let literal_type = Type::Tuple(literal_element_types(elements, symbols, ctx));

    // An excess property is a freshness failure, which the structural relation
    // the caller runs cannot see. tsc reports it as this same whole-value
    // failure, positioned on the property.
    if let Some(span) = excess_property_span
        && is_assignable_to(&literal_type, expected_type)
    {
        let diagnostic = Diagnostic::ts2322(
            &literal_type.name(),
            &expected_type.name(),
            ctx.file_name.clone(),
        );
        ctx.push(diagnostic.with_span(span));
        return InferredExpression::Unknown;
    }
    InferredExpression::Known(literal_type)
}

fn expected_tuple_name(expected_elements: &[Type], exact_optional_min_length: Option<usize>) -> String {
    match exact_optional_min_length {
        Some(min_length) => surge_ts_types::written_tuple_type(expected_elements.to_vec(), min_length).name(),
        None => Type::Tuple(expected_elements.to_vec()).name(),
    }
}

fn evaluate_tuple_literal_with_expected_type(
    elements: &[surge_ts_syntax::ParsedArrayElement],
    expected_elements: &[Type],
    exact_optional_min_length: Option<usize>,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    is_argument: bool,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let mut reported_element = false;
    let mut unresolved_element = false;
    let mut omitted_mismatch = false;
    for (index, element) in elements.iter().enumerate() {
        if index >= expected_elements.len() {
            if reported_element || unresolved_element {
                return InferredExpression::Unknown;
            }
            // The literal is longer than the tuple allows. tsc names the source by
            // its own widened element types (`Type '[number]' is not assignable to
            // type '[]'`); the hardcoded `unknown[]` this used to print named
            // neither side truthfully.
            // The extra elements are still checked expressions.
            for extra in &elements[index..] {
                let _ = evaluate_expression(&extra.expression, extra.span, symbols, ctx);
            }
            let diagnostics_before = ctx.diagnostics().len();
            let source_type_name =
                Type::Tuple(literal_element_types(elements, symbols, ctx)).name();
            ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
            let target_type_name = expected_tuple_name(expected_elements, exact_optional_min_length);
            // The lengths disagree, so no element is at fault: tsc reports the
            // whole value, as the argument it is or on the target it initializes.
            let (diagnostic, span) = if is_argument {
                (
                    Diagnostic::ts2345(&source_type_name, &target_type_name, ctx.file_name.clone()),
                    fallback_span,
                )
            } else {
                (
                    Diagnostic::ts2322(&source_type_name, &target_type_name, ctx.file_name.clone()),
                    target_span.or(fallback_span),
                )
            };
            let arity = format!(
                "  Source has {} element(s) but target allows only {}.",
                elements.len(),
                expected_elements.len()
            );
            ctx.push(diagnostic_with_syntax_span(crate::checks::relation_chain::with_chain(diagnostic, &[arity]), span));
            return InferredExpression::Unknown;
        }

        let optional_slot = exact_optional_min_length.is_some_and(|min_length| index >= min_length);
        if optional_slot && element.omitted {
            continue;
        }
        // Elementwise elaboration reads the slot (`getIndexedAccessType`),
        // which an optional element widens with `undefined`.
        let read_slot;
        let expected_element_type = if optional_slot {
            read_slot = surge_ts_types::union_type(vec![expected_elements[index].clone(), Type::Undefined]);
            &read_slot
        } else {
            &expected_elements[index]
        };
        // The contextual type is the element's own (`removeMissingType` in
        // `getTypeOfPropertyOfContextualType`); only the relation reads the slot.
        let inferred_element = evaluate_expression_with_expected_type(
            &element.expression,
            element.span,
            Some(&expected_elements[index]),
            ExpectedTypeDiagnostic::TypeNotAssignable,
            symbols,
            ctx,
        );

        match inferred_element {
            InferredExpression::Known(actual_type) => {
                if actual_type.is_unknown()
                    || crate::checks::call::is_open_instantiation(&actual_type)
                {
                    continue;
                }

                if element.omitted && !is_assignable_to(&actual_type, expected_element_type) {
                    omitted_mismatch = true;
                    continue;
                }
                if !is_assignable_to(&actual_type, expected_element_type) {
                    let actual_type =
                        crate::checks::expr::widen_type_for_contextual_type(&actual_type, expected_element_type);
                    let reported_target = crate::checks::expr::reported_relation_target(
                        &actual_type,
                        expected_element_type,
                    );
                    let actual_type_name =
                        crate::checks::expr::source_display_name(&actual_type, &reported_target);
                    let expected_type_name = reported_target.name();
                    let diagnostic = crate::checks::expr::type_not_assignable_diagnostic(
                        &actual_type,
                        &reported_target,
                        &actual_type_name,
                        &expected_type_name,
                        ctx.file_name.clone(),
                    );

                    ctx.push(diagnostic_with_syntax_span(
                        diagnostic,
                        choose_span(element.span, fallback_span),
                    ));
                    // `elaborateElementwise` reports every failing element.
                    reported_element = true;
                    continue;
                }
                if optional_slot && !is_assignable_to(&actual_type, &expected_elements[index]) {
                    omitted_mismatch = true;
                }
            }
            // `elaborateElementwise` goes on to the remaining elements after one
            // it could not relate.
            InferredExpression::UnresolvedIdentifier { .. }
            | InferredExpression::MissingProperty { .. }
            | InferredExpression::Unknown => {
                unresolved_element = true;
            }
        }
    }
    if reported_element || unresolved_element {
        return InferredExpression::Unknown;
    }

    // A literal may stop short of trailing slots that accept `undefined` —
    // how an optional element (`[string[], Opts?]`) is represented.
    let trailing_optional = match exact_optional_min_length {
        Some(min_length) => elements.len() >= min_length,
        None => expected_elements[elements.len().min(expected_elements.len())..]
            .iter()
            .all(|slot| is_assignable_to(&Type::Undefined, slot)),
    };
    // A mismatch only an omitted element carries is reported on the whole
    // literal, as a length mismatch is.
    if omitted_mismatch || (elements.len() != expected_elements.len() && !trailing_optional) {
        let source_type_name = Type::Tuple(literal_element_types(elements, symbols, ctx)).name();
        let target_type_name = expected_tuple_name(expected_elements, exact_optional_min_length);
        let (diagnostic, span) = if is_argument {
            (
                Diagnostic::ts2345(&source_type_name, &target_type_name, ctx.file_name.clone()),
                fallback_span,
            )
        } else {
            (
                Diagnostic::ts2322(&source_type_name, &target_type_name, ctx.file_name.clone()),
                target_span.or(fallback_span),
            )
        };
        // `propertiesRelatedTo`'s tuple arity line for a literal too short.
        let diagnostic = if !omitted_mismatch && elements.len() < expected_elements.len() {
            let minimum = exact_optional_min_length.unwrap_or_else(|| {
                expected_elements
                    .iter()
                    .rposition(|slot| !is_assignable_to(&Type::Undefined, slot))
                    .map_or(0, |last| last + 1)
            });
            crate::checks::relation_chain::with_chain(
                diagnostic,
                &[format!("  Source has {} element(s) but target requires {minimum}.", elements.len())],
            )
        } else {
            diagnostic
        };
        ctx.push(diagnostic_with_syntax_span(diagnostic, span));
        return InferredExpression::Unknown;
    }

    // The literal writes each of its elements, so they are all required even
    // where a slot admits `undefined` (`[""]` against `[string | undefined]`).
    InferredExpression::Known(surge_ts_types::written_tuple_type(expected_elements.to_vec(), elements.len()))
}

/// Whether an expected member type stands at surge's degradation sentinel, so a
/// comparison against it proves nothing. Deliberately shallow: this runs for
/// every property of every checked object literal, and the deep walk resolves
/// lazy references — too expensive here, and re-entrant while a literal is
/// mid-check.
fn expected_member_is_degraded(ty: &Type) -> bool {
    match ty {
        Type::Unknown => true,
        Type::Union(union) => union.types().iter().any(|member| member.is_unknown()),
        _ => false,
    }
}

/// `hasExcessProperties` skips a target the global `Object` is a subset of:
/// every object is an `Object`, so none of a literal's properties is excess.
fn is_global_object_interface(object: &surge_ts_types::ObjectType) -> bool {
    const OBJECT_MEMBERS: [&str; 7] = [
        "constructor",
        "toString",
        "toLocaleString",
        "valueOf",
        "hasOwnProperty",
        "isPrototypeOf",
        "propertyIsEnumerable",
    ];
    object.alias_name.as_deref() == Some("Object")
        && object.properties.keys().all(|name| OBJECT_MEMBERS.contains(&name.as_ref()))
}

/// tsc's excess-property report: the first property the target does not
/// declare, reported once. It runs only after the written properties have
/// checked out — a property that fails against its expected type reports
/// instead — and it takes precedence over the missing-required report.
/// Only an empty target (`isEmptyResolvedType`: no properties, signatures or
/// index signatures) accepts every property; a number index answers the
/// numeric names alone (`isKnownProperty`, `getApplicableIndexInfoForName`).
/// A computed key surge could not name stays out of a target that declares
/// no property: tsc turns a key typed as a union or enum into an index
/// signature of the literal (`checkObjectLiteral`), which is never excess.
pub(crate) fn report_excess_property(
    properties: &[ParsedObjectProperty],
    expected_object_type: &surge_ts_types::ObjectType,
    fallback_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    if ctx.relating_unfresh_return {
        return false;
    }
    let is_empty_target = expected_object_type.properties.is_empty()
        && expected_object_type.number_index_type.is_none()
        && expected_object_type.call_signature().is_none()
        && expected_object_type.construct_signature().is_none();
    let keyed = expected_object_type.active_keyed_indexes();
    if keyed.is_none()
        && (expected_object_type.allows_string_index_access() || is_empty_target)
        || is_global_object_interface(expected_object_type)
    {
        return false;
    }
    let Some(property) = properties.iter().find(|property| {
        !property.is_spread
            && !expected_object_type.contains_property(&property.name)
            && !keyed.is_some_and(|keyed| {
                property.computed_key.is_some()
                    || keyed.read(&Type::StringLiteral(property.name.clone())).is_some()
            })
            && !(property.computed_key.is_some() && expected_object_type.symbol_index_type().is_some())
            && !(expected_object_type.number_index_type.is_some()
                && crate::program::is_numeric_literal_name(&property.name))
            && !(expected_object_type.properties.is_empty()
                && property.computed_key.is_some()
                && !computed_key_names_symbol(property, symbols, ctx))
    }) else {
        return false;
    };

    let target = Type::Object(with_type_copy_reason(TypeCopyReason::ExpectedType, || {
        expected_object_type.clone()
    }));
    let diagnostic = excess_property_diagnostic(property, &target, &target.name(), ctx);
    ctx.push(diagnostic_with_syntax_span(
        diagnostic,
        choose_span(
            property.name_span,
            choose_span(property.span, fallback_span),
        ),
    ));
    true
}

/// A computed key that names a property of the literal rather than giving it
/// an index signature: a unique symbol (`[sym]`, `[Symbol.iterator]`), which
/// no number index signature covers.
fn computed_key_names_symbol(property: &ParsedObjectProperty, symbols: &SymbolTable, ctx: &mut CheckerContext) -> bool {
    let Some(key) = property.computed_key.as_deref() else {
        return false;
    };
    if let ParsedExpression::PropertyAccess { object, .. } = key
        && matches!(object.as_ref(), ParsedExpression::Identifier { name, .. } if name == "Symbol")
    {
        return true;
    }
    let checkpoint = ctx.diagnostics().len();
    let key_type = crate::infer::infer_expression(key, symbols, ctx);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    matches!(key_type, InferredExpression::Known(Type::Reference(reference)) if reference.is_unique_symbol())
}

/// tsc's excess-property report (`propertiesRelatedTo`): a property the
/// literal names with an identifier, of which a member of the target is a
/// likely misspelling (`getSuggestionForNonexistentProperty`), is TS2561
/// naming that member; any other is TS2353.
fn excess_property_diagnostic(
    property: &ParsedObjectProperty,
    target: &Type,
    target_name: &str,
    ctx: &CheckerContext,
) -> Diagnostic {
    let file_name = ctx.file_name.clone();
    let name = written_object_property_name(property);
    // The parsed property keeps no key kind, so an identifier is told from a
    // string-literal key by its span: only the identifier spans exactly its name.
    let written_as_identifier = property.computed_key.is_none()
        && property
            .name_span
            .is_some_and(|span| span.end.saturating_sub(span.start) == property.name.len())
        && property.name.chars().next().is_some_and(|first| first.is_alphabetic() || first == '_' || first == '$')
        && property.name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    match written_as_identifier
        .then(|| crate::checks::expr::property_spelling_suggestion(&property.name, target))
        .flatten()
    {
        Some(suggestion) => Diagnostic::ts2561(&name, target_name, suggestion, file_name),
        None => Diagnostic::ts2353(&name, target_name, file_name),
    }
}

/// An object literal property named as written (`'three'` keeps its quotes).
fn written_object_property_name(property: &ParsedObjectProperty) -> String {
    crate::checks::expr::spelled_property_name(&property.name, property.written_name.as_deref())
        .map_or_else(|| property.name.clone(), |written| written.to_string())
}

fn evaluate_object_literal_with_expected_type(
    properties: &[ParsedObjectProperty],
    expected_object_type: &surge_ts_types::ObjectType,
    fallback_span: Option<SyntaxTextSpan>,
    target_span: Option<SyntaxTextSpan>,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    // Taken, not read: it describes this literal only, and the properties
    // evaluated below are literals of their own.
    let union_target = ctx.union_literal_target.take();
    crate::checks::expr::check_computed_property_keys(
        properties,
        fallback_span,
        Some(expected_object_type),
        symbols,
        ctx,
    );
    let computed_index_signatures =
        crate::infer::expression::computed_key_index_signatures(properties, symbols, ctx);
    let properties =
        &*crate::infer::expression::resolve_computed_property_names(properties, symbols, ctx);
    let closure_symbols =
        crate::infer::expression::object_member_closure_symbols(properties, symbols);
    let member_symbols = closure_symbols.as_ref().unwrap_or(symbols);
    // An intersection may carry a `ThisType<T>` marker, which surge does not
    // model, and a contextual type still mentioning a type parameter is
    // mid-inference: there the members keep the `this` they had.
    let contextual_this = (!expected_object_type.is_intersection
        && !expected_object_type.synthetic_open_index)
        .then(|| {
            crate::infer::expression::contextual_literal_member_this(
                properties,
                || Type::Object(expected_object_type.clone()),
                ctx,
            )
        })
        .flatten()
        .filter(|member_this| {
            !mentions_type_parameter(member_this.ty())
                && !crate::checks::function::type_contains_degradation(member_this.ty())
        });
    let object_start = Instant::now();
    let mut inferred_property_types = BTreeMap::new();
    // The empty object type `{}` (no properties, no string index) accepts any
    // object literal without excess-property errors, matching tsc. Library
    // signatures like `Object.keys(o: {})` rely on this.
    // A `{ ...source }` spread contributes `source`'s properties under an empty
    // name; it is neither an excess key nor a single checkable property here, and
    // it may supply required properties we don't track by name. Skip spreads in
    // the excess-property scan and, when any spread is present, in the missing
    // required-property scan below (conservative: under-check rather than emit a
    // false `TS2353`/`TS2741`).
    let has_spread = properties.iter().any(|property| property.is_spread);
    // A property a later spread always writes is not a property of the
    // literal's type (`getSpreadType`), so `elaborateObjectLiteral` relates
    // the spread's member there, never the value written before it.
    let overwritten_by_spread: Vec<bool> = if has_spread {
        let mut spreads_after: Vec<Type> = Vec::new();
        let mut overwritten = vec![false; properties.len()];
        for (index, property) in properties.iter().enumerate().rev() {
            if property.is_spread {
                if property.unnamed_key_value.is_none()
                    && let InferredExpression::Known(spread) =
                        crate::infer::infer_expression(&property.value, symbols, ctx)
                {
                    spreads_after.push(spread);
                }
            } else if !property.is_accessor && property.computed_key.is_none() {
                overwritten[index] = spreads_after
                    .iter()
                    .any(|spread| super::expr::spread_always_writes(spread, &property.name));
            }
        }
        overwritten
    } else {
        Vec::new()
    };

    // Set when a property the literal *writes* is compared against an expected
    // member surge could not model. tsc reports the written properties that
    // fail, each at the property, and then nothing else for the literal: the
    // missing-required-property report never happens. When the member is a
    // degradation sentinel the comparison passes permissively, so a missing
    // property below would be reported *instead* of the property error tsc
    // reports — the `@ts-expect-error` a test wrote over the property then does
    // not cover it. Withhold the missing-property report in that case.
    let mut degraded_property_comparison = false;
    let mut exact_optional_mismatch = false;
    let mut explicit_properties: Vec<(&str, Option<SyntaxTextSpan>)> = Vec::new();
    // `elaborateObjectLiteral` relates every property and reports each one
    // that fails (`elaborateElement`); once one has, the literal as a whole is
    // not reported (no excess or missing property).
    let mut elaborated_property_failure = false;

    for (property_index, property) in properties.iter().enumerate() {
        let property_symbols = if matches!(property.value, ParsedExpression::ArrowFunction(_)) {
            member_symbols
        } else {
            symbols
        };
        if property.is_spread {
            if property.unnamed_key_value.is_none() && !explicit_properties.is_empty() {
                let spread = crate::infer::infer_expression(&property.value, symbols, ctx);
                super::expr::report_overwritten_properties(&spread, &explicit_properties, ctx);
            }
            continue;
        }
        if !property.is_accessor && property.computed_key.is_none() {
            explicit_properties.push((&property.name, property.span.or(property.name_span)));
        }
        if overwritten_by_spread.get(property_index).copied().unwrap_or(false) {
            let contextual = expected_object_type.get_property_type(&property.name).cloned();
            let _ = evaluate_expression_with_expected_type(
                &property.value,
                property.value_span.or(property.span),
                contextual.as_ref(),
                ExpectedTypeDiagnostic::ContextOnly,
                symbols,
                ctx,
            );
            continue;
        }
        record_object_literal_property_check();
        // `getIndexedAccessTypeOrUndefined(target, nameType)`: a numeric name
        // prefers the number index signature (`findApplicableIndexInfo`).
        let expected_property = if let Some(expected_property) =
            expected_object_type.get_property(&property.name).cloned()
        {
            expected_property
        } else if let Some(index_type) = match expected_object_type.keyed_index_for_name(&property.name) {
            Some(indexed) => indexed,
            None => expected_object_type
                .applicable_index_type(crate::program::is_numeric_literal_name(&property.name))
                .cloned(),
        } {
            ObjectProperty::required(index_type)
        } else {
            // The target has no such property, so there is no contextual type
            // to check the value against — but the value is still an expression
            // with errors of its own, and tsc reports them alongside the excess
            // report below. A *method* is checked here too: tsc's
            // `checkObjectLiteral` walks every property, and a method whose name
            // the contextual type does not declare gets no contextual signature,
            // so its unannotated parameters are implicit `any`
            // (`useOpen({ onError(err) {} })` against `o: {}` is a TS7006 tsc
            // reports and skipping the property silenced).
            if property.is_shorthand {
                ctx.shorthand_property_depth += 1;
            }
            let _ = evaluate_expression(
                &property.value,
                property.value_span.or(property.span),
                symbols,
                ctx,
            );
            if property.is_shorthand {
                ctx.shorthand_property_depth -= 1;
            }
            continue;
        };

        let expected_property_type = if expected_property.is_optional() {
            surge_ts_types::union_type(vec![
                with_type_copy_reason(TypeCopyReason::ExpectedType, || {
                    expected_property.ty.clone()
                }),
                Type::Undefined,
            ])
        } else {
            with_type_copy_reason(TypeCopyReason::ExpectedType, || {
                expected_property.ty.clone()
            })
        };
        if expected_member_is_degraded(&expected_property.ty) {
            degraded_property_comparison = true;
        }

        // Without `exactOptionalPropertyTypes` an optional property's type
        // *includes* `undefined` — tsc folds it in at declaration resolution —
        // so the contextual type has to carry it. A conditional value is checked
        // branch by branch against the contextual type, and `flag ? x : undefined`
        // fails on its `undefined` branch otherwise.
        // With it, `getTypeOfPropertyOfContextualType` removes the missing type
        // an optional property carries.
        let contextual_property_type = if expected_property.is_optional()
            && surge_ts_types::strict_null_checks()
            && surge_ts_types::exact_optional_property_types()
        {
            with_type_copy_reason(TypeCopyReason::ExpectedType, || expected_property.ty.clone())
        } else {
            expected_property_type.clone()
        };

        // A `get`/`set` accessor is written as a function but *is* the property:
        // contextually type it as one returning the expected type, then compare
        // the accessor's value type (getter return / setter parameter) rather
        // than the accessor function itself.
        let accessor_contextual_type = property.is_accessor.then(|| {
            Type::Function(surge_ts_types::FunctionType::new(
                Vec::new(),
                with_type_copy_reason(TypeCopyReason::ExpectedType, || {
                    contextual_property_type.clone()
                }),
                false,
                0,
            ))
        });
        if property.is_shorthand {
            ctx.shorthand_property_depth += 1;
        }
        // A member declared with method syntax relates its parameters
        // bivariantly, which only this property check knows: the value is
        // typed by it and related below.
        let method_target = !property.is_accessor
            && expected_property.is_method()
            && matches!(expected_property.ty.peeled(), Type::Function(_));
        // A contextually-typed arrow whose returns do not fit is one
        // whole-signature mismatch anchored on the property name. A nested
        // literal that fails as a whole — a missing property, not one of its
        // own members — is reported on this property's *name* too, which is
        // where tsc's `elaborateObjectLiteral` stops descending.
        let diagnostics_before_property = ctx.diagnostics().len();
        if let (Some(member_this), ParsedExpression::ArrowFunction(member)) = (&contextual_this, &property.value) {
            member_this.hand_to(member, property.is_method || property.is_accessor, ctx);
        }
        let inferred_property = evaluate_expression_with_expected_type_anchored(
            &property.value,
            property.value_span.or(property.span),
            (!property.is_accessor).then_some(property.name_span).flatten(),
            Some(
                accessor_contextual_type
                    .as_ref()
                    .unwrap_or(&contextual_property_type),
            ),
            // A literal that is only contextually typed relates none of its
            // members either.
            if method_target || expected_diagnostic == ExpectedTypeDiagnostic::ContextOnly {
                ExpectedTypeDiagnostic::ContextOnly
            } else {
                ExpectedTypeDiagnostic::TypeNotAssignable
            },
            property_symbols,
            ctx,
        );
        ctx.next_arrow_this = None;
        if property.is_shorthand {
            ctx.shorthand_property_depth -= 1;
        }
        if let ParsedExpression::ArrowFunction(getter) = &property.value {
            crate::infer::expression::check_paired_setter(property, getter, member_symbols, ctx);
        }
        let inferred_property = match inferred_property {
            InferredExpression::Known(Type::Function(function_type)) if property.is_accessor => {
                InferredExpression::Known(match function_type.parameters().first() {
                    Some(parameter) => parameter.clone(),
                    None => function_type.return_type().clone(),
                })
            }
            other => other,
        };

        match inferred_property {
            InferredExpression::Known(actual_type) => {
                if actual_type.is_unknown()
                    || crate::checks::call::is_open_instantiation(&actual_type)
                {
                    inferred_property_types.insert(property.name.clone(), Type::Unknown);
                    continue;
                }

                inferred_property_types.insert(
                    property.name.clone(),
                    with_type_copy_reason(TypeCopyReason::ExpectedType, || actual_type.clone()),
                );
                record_assignability_check();
                let assignable = if method_target && actual_type != Type::Undefined {
                    surge_ts_types::is_member_assignable_to(&actual_type, &expected_property.ty, true)
                } else {
                    is_assignable_to(&actual_type, &expected_property_type)
                };
                // Under `exactOptionalPropertyTypes` a value only its `undefined`
                // keeps out of the declared type still relates to the property's
                // read type (`undefined` to tsc's missing type), so tsc does not
                // elaborate into it: the literal as a whole is reported below.
                if assignable
                    && !method_target
                    && crate::checks::expr::is_exact_optional_slot(
                        &expected_property.ty,
                        expected_property.is_optional(),
                    )
                    && !is_assignable_to(&actual_type, &expected_property.ty)
                {
                    exact_optional_mismatch = true;
                }
                if expected_diagnostic != ExpectedTypeDiagnostic::ContextOnly && !assignable {
                    // The comparison target carries the optionality-implied
                    // `undefined`, but tsc's elaboration names the property's
                    // written type — a value that is not `undefined` failed
                    // against that, not against the widened union.
                    let reported_target = if actual_type == Type::Undefined {
                        &expected_property_type
                    } else {
                        &expected_property.ty
                    };
                    // `checkExpressionForMutableLocation`: the literal's own
                    // type of a property value is widened unless its target
                    // is a literal of the same kind.
                    let actual_type = crate::checks::expr::widen_type_for_contextual_type(
                        &actual_type,
                        &expected_property.ty,
                    );
                    let actual_type_name = source_display_name(&actual_type, reported_target);
                    let expected_type_name = reported_target.name();
                    let (actual_type_name, expected_type_name) =
                        crate::checks::expr::disambiguated_pair(
                            &actual_type,
                            actual_type_name,
                            reported_target,
                            expected_type_name,
                            &ctx.file_name,
                        );
                    // A name that is not a literal (`[key]`, `[-1]`) gets its own
                    // message, reported on the whole `[…]` name
                    // (`elaborateObjectLiteral`, `IsComputedNonLiteralName`).
                    if property.computed_key.is_some() {
                        ctx.push(diagnostic_with_syntax_span(
                            Diagnostic::ts2418(
                                &actual_type_name,
                                &expected_type_name,
                                ctx.file_name.clone(),
                            ),
                            choose_span(
                                property.name_span,
                                choose_span(property.span, fallback_span),
                            ),
                        ));
                    } else {
                        let diagnostic = crate::checks::expr::type_not_assignable_diagnostic(
                            &actual_type,
                            &expected_property_type,
                            &actual_type_name,
                            &expected_type_name,
                            ctx.file_name.clone(),
                        );

                        ctx.push(diagnostic_with_syntax_span(
                            diagnostic,
                            choose_span(
                                property.name_span,
                                choose_span(
                                    property.value_span,
                                    choose_span(property.span, fallback_span),
                                ),
                            ),
                        ));
                    }
                    elaborated_property_failure = true;
                }
            }
            // A name or member that did not resolve is tsc's error type, an
            // `any` that fits the member, and the literal is still compared.
            InferredExpression::UnresolvedIdentifier { .. } | InferredExpression::MissingProperty { .. }
                if !matches!(
                    property.value,
                    ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }
                ) =>
            {
                inferred_property_types.insert(property.name.clone(), Type::Any);
            }
            InferredExpression::UnresolvedIdentifier { .. }
            | InferredExpression::MissingProperty { .. }
            | InferredExpression::Unknown => {
                inferred_property_types.insert(property.name.clone(), Type::Unknown);
                // A nested literal that reported its own failure is one
                // property `elaborateObjectLiteral` elaborated into; it goes
                // on to the next.
                if expected_diagnostic != ExpectedTypeDiagnostic::ContextOnly
                    && matches!(
                        property.value,
                        ParsedExpression::ObjectLiteral { .. } | ParsedExpression::ArrayLiteral { .. }
                    )
                    && ctx.diagnostics().len() > diagnostics_before_property
                {
                    elaborated_property_failure = true;
                    continue;
                }
                // The value's own error is already reported and the literal
                // cannot be compared further, but an excess property is a
                // separate report tsc still makes — unless an earlier property
                // already failed, which is all tsc reports for the literal.
                if expected_diagnostic != ExpectedTypeDiagnostic::ContextOnly
                    && !elaborated_property_failure
                {
                    report_excess_property(properties, expected_object_type, fallback_span, symbols, ctx);
                }
                return InferredExpression::Unknown;
            }
        }
    }

    if elaborated_property_failure {
        record_object_literal_own_type(properties, &inferred_property_types, expected_object_type, fallback_span, ctx);
        return InferredExpression::Unknown;
    }

    // An assertion is related by comparability, so an extra member is no error.
    if expected_diagnostic != ExpectedTypeDiagnostic::ContextOnly
        && report_excess_property(properties, expected_object_type, fallback_span, symbols, ctx)
    {
        record_object_literal_own_type(properties, &inferred_property_types, expected_object_type, fallback_span, ctx);
        return InferredExpression::Unknown;
    }

    // `checkObjectLiteral` folds every spread into the literal's type
    // (`getSpreadType`) and the whole is related to the target; a spread member
    // the target rejects is no written property `elaborateObjectLiteral` can
    // descend into, so the literal is reported as a whole.
    if has_spread
        && !degraded_property_comparison
        && union_target.is_none()
        && matches!(
            expected_diagnostic,
            ExpectedTypeDiagnostic::TypeNotAssignable | ExpectedTypeDiagnostic::ArgumentNotAssignable
        )
    {
        let literal = ParsedExpression::ObjectLiteral {
            properties: properties.to_vec(),
            span: fallback_span,
        };
        let diagnostics_before = ctx.diagnostics().len();
        let spread_type = crate::infer::infer_expression(&literal, symbols, ctx);
        ctx.truncate_diagnostics(diagnostics_before);
        let expected = Type::Object(with_type_copy_reason(TypeCopyReason::ExpectedType, || {
            expected_object_type.clone()
        }));
        if let InferredExpression::Known(spread_type) = spread_type
            && matches!(spread_type, Type::Object(_))
            && !crate::checks::function::type_contains_unknown(&spread_type)
            && !crate::checks::function::type_contains_unknown(&expected)
            && !is_assignable_to(&spread_type, &expected)
        {
            let reported_target = crate::checks::expr::reported_relation_target(&spread_type, &expected);
            let source_name = crate::checks::expr::source_display_name(&spread_type, &reported_target);
            let diagnostic = crate::checks::expr::assignability_mismatch_diagnostic(
                &spread_type,
                &reported_target,
                &source_name,
                &reported_target.name(),
                expected_diagnostic == ExpectedTypeDiagnostic::ArgumentNotAssignable,
                ctx.file_name.clone(),
            );
            ctx.push(diagnostic_with_syntax_span(
                diagnostic,
                choose_span(target_span, fallback_span),
            ));
            return InferredExpression::Unknown;
        }
    }

    let missing_property_names: Vec<String> = if has_spread || degraded_property_comparison {
        Vec::new()
    } else {
        expected_object_type
            .required_properties()
            .filter(|(property_name, _)| {
                // The literal's apparent type answers the global `Object` members.
                surge_ts_types::object_prototype_member_type(property_name).is_none()
                    && !properties
                        .iter()
                        .any(|property| property.name == property_name.as_ref())
            })
            .map(|(property_name, _)| property_name.to_string())
            .collect()
    };

    // An asserted literal may omit members freely (`{} as Ctx` is not an error).
    if expected_diagnostic != ExpectedTypeDiagnostic::ContextOnly
        && let Some(property_name) = missing_property_names.first()
    {
        // tsc widens a literal member only where its contextual type lets it:
        // `kind: "two"` against a `"one" | "two"` slot stays `"two"`, since the
        // widened `string` would no longer fit.
        let displayed_property_types: BTreeMap<String, Type> = inferred_property_types
            .iter()
            .map(|(name, ty)| {
                let widened = crate::checks::expr::widen_type(ty);
                let keeps_literal = expected_object_type
                    .properties
                    .get(name.as_str())
                    .is_some_and(|expected| {
                        is_assignable_to(ty, &expected.ty)
                            && !is_assignable_to(&widened, &expected.ty)
                    });
                (
                    name.clone(),
                    if keeps_literal { ty.clone() } else { widened },
                )
            })
            .collect();
        let source_type_name =
            object_literal_source_type_name(properties, &displayed_property_types).name();
        let target_type_name =
            Type::Object(with_type_copy_reason(TypeCopyReason::ExpectedType, || {
                expected_object_type.clone()
            }))
            .name();

        // tsc surfaces a missing required property differently for an
        // intersection target: it reports the outer assignability code (the
        // missing property becomes nested elaboration) rather than the
        // standalone TS2741. Mirror that so the reported code matches. A union
        // target is the same, and it is the union tsc names.
        let target_type_name = union_target
            .as_ref()
            .map_or(target_type_name, |union| union.name());
        let diagnostic = if expected_object_type.is_intersection || union_target.is_some() {
            match expected_diagnostic {
                ExpectedTypeDiagnostic::TypeNotAssignable | ExpectedTypeDiagnostic::ContextOnly => {
                    Diagnostic::ts2322(&source_type_name, &target_type_name, ctx.file_name.clone())
                }
                ExpectedTypeDiagnostic::ArgumentNotAssignable => {
                    Diagnostic::ts2345(&source_type_name, &target_type_name, ctx.file_name.clone())
                }
                ExpectedTypeDiagnostic::SatisfiesNotAssignable => {
                    Diagnostic::ts1360(&source_type_name, &target_type_name, ctx.file_name.clone())
                }
            }
        } else {
            crate::checks::expr::missing_properties_diagnostic(
                property_name,
                &missing_property_names,
                &source_type_name,
                &target_type_name,
                &ctx.file_name,
            )
        };

        ctx.push(diagnostic_with_syntax_span(
            diagnostic,
            choose_span(target_span, fallback_span),
        ));
        record_object_literal_own_type(properties, &inferred_property_types, expected_object_type, fallback_span, ctx);
        return InferredExpression::Unknown;
    }

    // A computed member keyed by an index signature has no name to elaborate
    // at (`elaborateObjectLiteral`), so the literal as a whole relates through
    // the index signature it carries.
    if expected_diagnostic != ExpectedTypeDiagnostic::ContextOnly
        && !elaborated_property_failure
        && !has_spread
        && let Some((string_index, number_index)) = computed_index_signatures
    {
        let source_properties: surge_ts_types::PropertyMap = inferred_property_types
            .iter()
            .map(|(name, ty)| (std::sync::Arc::<str>::from(name.as_str()), surge_ts_types::ObjectProperty::required(ty.clone())))
            .collect();
        let source = crate::checks::expr::widen_type(&Type::Object(
            crate::metrics::alloc_object_type(source_properties, string_index).with_number_index_type(number_index),
        ));
        let target = Type::Object(expected_object_type.clone());
        if !is_assignable_to(&source, &target) {
            let target_type_name = union_target.as_ref().map_or_else(|| target.name(), |union| union.name());
            let diagnostic = match expected_diagnostic {
                ExpectedTypeDiagnostic::ArgumentNotAssignable => {
                    Diagnostic::ts2345(source.name(), &target_type_name, ctx.file_name.clone())
                }
                ExpectedTypeDiagnostic::SatisfiesNotAssignable => {
                    Diagnostic::ts1360(source.name(), &target_type_name, ctx.file_name.clone())
                }
                _ => Diagnostic::ts2322(source.name(), &target_type_name, ctx.file_name.clone()),
            };
            let reported_target = union_target.clone().unwrap_or(target);
            let diagnostic = crate::checks::relation_chain::with_chain(
                diagnostic,
                &crate::checks::relation_chain::chain_beneath_head(&source, &reported_target),
            );
            ctx.push(diagnostic_with_syntax_span(diagnostic, choose_span(target_span, fallback_span)));
            return InferredExpression::Unknown;
        }
    }

    // `reportRelationError` heads the literal's report TS2375, or TS2379 for an
    // argument, when that is what fails; an assertion or `satisfies` keeps its
    // own head.
    if exact_optional_mismatch
        && matches!(
            expected_diagnostic,
            ExpectedTypeDiagnostic::TypeNotAssignable | ExpectedTypeDiagnostic::ArgumentNotAssignable
        )
    {
        let displayed_property_types: BTreeMap<String, Type> = inferred_property_types
            .iter()
            .map(|(name, ty)| (name.clone(), crate::checks::expr::widen_type(ty)))
            .collect();
        let source_type = object_literal_source_type_name(properties, &displayed_property_types);
        let source_type_name = source_type.name();
        let target_type = match union_target.as_ref() {
            Some(union) => union.clone(),
            None => Type::Object(with_type_copy_reason(TypeCopyReason::ExpectedType, || {
                expected_object_type.clone()
            })),
        };
        let target_type_name = target_type.name();
        let diagnostic = if expected_diagnostic == ExpectedTypeDiagnostic::ArgumentNotAssignable {
            Diagnostic::ts2379(&source_type_name, &target_type_name, ctx.file_name.clone())
        } else {
            Diagnostic::ts2375(&source_type_name, &target_type_name, ctx.file_name.clone())
        };
        let diagnostic = crate::checks::relation_chain::with_chain(
            diagnostic,
            &crate::checks::relation_chain::chain_beneath_head(&source_type, &target_type),
        );
        ctx.push(diagnostic_with_syntax_span(
            diagnostic,
            choose_span(target_span, fallback_span),
        ));
        return InferredExpression::Unknown;
    }

    record_object_literal_own_type(properties, &inferred_property_types, expected_object_type, fallback_span, ctx);
    let result = InferredExpression::Known(Type::Object(with_type_copy_reason(
        TypeCopyReason::ExpectedType,
        || expected_object_type.clone(),
    )));
    record_program_timing(ctx.timings.as_ref(), |timings| {
        timings.object_literal_checking += object_start.elapsed()
    });
    result
}

/// The compiler API's type for an object literal checked against an expected
/// object: the literal's own properties, in source order, each widened as
/// tsc's `checkExpressionForMutableLocation` widens it unless the member the
/// literal is contextually typed by keeps literals. A literal the check left
/// before reaching every member has no own type recorded.
fn record_object_literal_own_type(
    properties: &[ParsedObjectProperty],
    inferred_property_types: &BTreeMap<String, Type>,
    expected_object_type: &surge_ts_types::ObjectType,
    literal_span: Option<SyntaxTextSpan>,
    ctx: &CheckerContext,
) {
    if !crate::semantic::recording() {
        return;
    }
    let mut own = surge_ts_types::PropertyMap::default();
    for property in properties.iter().filter(|property| !property.is_spread) {
        let Some(ty) = inferred_property_types.get(&property.name) else { return };
        let keeps_literal = expected_object_type
            .properties
            .get(property.name.as_str())
            .is_some_and(|expected| contains_literal_type(&expected.ty.peeled()));
        let ty = if keeps_literal { ty.clone() } else { crate::checks::expr::widen_type(ty) };
        own.insert(
            property.name.as_str().into(),
            surge_ts_types::ObjectProperty::required(ty).with_method(property.is_method),
        );
    }
    let own = Type::Object(surge_ts_types::ObjectType::new(own, None));
    crate::semantic::record_literal_type(literal_span, &own, ctx);
}

fn contains_literal_type(ty: &Type) -> bool {
    match ty {
        Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().any(contains_literal_type),
        _ => false,
    }
}

fn object_literal_source_type_name(
    properties: &[ParsedObjectProperty],
    inferred_property_types: &BTreeMap<String, Type>,
) -> Type {
    let properties = properties
        .iter()
        .map(|property| {
            let ty = inferred_property_types
                .get(&property.name)
                .cloned()
                .unwrap_or(Type::Unknown);
            (property.name.as_str().into(), ObjectProperty::required(ty).with_method(property.is_method))
        })
        .collect::<surge_ts_types::PropertyMap>();

    Type::Object(alloc_object_type(properties, None))
}

fn evaluate_conditional_expression_with_expected_type(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    expected_type: &Type,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    let ParsedExpression::Conditional {
        condition,
        condition_span,
        when_true,
        when_true_span,
        when_false,
        when_false_span,
        truthiness_tests,
    } = expression
    else {
        return evaluate_expression(expression, fallback_span, symbols, ctx);
    };
    crate::checks::function::report_unreferenced_callable_conditions(truthiness_tests, condition, symbols, ctx);

    // Narrow a discriminated union for each branch (`x.kind === "a" ? … : …`).
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
    // A guard on an element access (`typeof xs[0] === 'string' ? xs[0] : …`)
    // narrows the access itself, which no binding's type can carry.
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
    let true_symbols = true_symbols.as_ref().unwrap_or(symbols);
    let false_symbols = false_symbols.as_ref().unwrap_or(symbols);

    if *expected_type == Type::Any {
        let _ = evaluate_expression(condition, condition_span.or(fallback_span), symbols, ctx);
        let _ = evaluate_expression(
            when_true,
            when_true_span.or(fallback_span),
            true_symbols,
            ctx,
        );
        let _ = evaluate_expression(
            when_false,
            when_false_span.or(fallback_span),
            false_symbols,
            ctx,
        );

        return InferredExpression::Known(Type::Any);
    }

    let split_branches = std::mem::take(&mut ctx.split_returned_conditional);
    let condition_result =
        evaluate_expression(condition, condition_span.or(fallback_span), symbols, ctx);
    if split_branches {
        let true_mismatch = check_returned_conditional_branch(
            when_true,
            when_true_span.or(fallback_span),
            expected_type,
            expected_diagnostic,
            true_symbols,
            ctx,
        );
        let false_mismatch = check_returned_conditional_branch(
            when_false,
            when_false_span.or(fallback_span),
            expected_type,
            expected_diagnostic,
            false_symbols,
            ctx,
        );
        if true_mismatch || false_mismatch || matches!(condition_result, InferredExpression::Unknown)
        {
            return InferredExpression::Unknown;
        }
        return InferredExpression::Known(with_type_copy_reason(TypeCopyReason::ExpectedType, || {
            expected_type.clone()
        }));
    }
    // tsc types each branch under the contextual type but never elaborates a
    // mismatch into a branch (`elaborateError` has no conditional case): the
    // union of the branches is related to the target once, where the caller
    // anchors it.
    let true_result = evaluate_conditional_branch(
        when_true,
        when_true_span.or(fallback_span),
        expected_type,
        expected_diagnostic,
        true_symbols,
        ctx,
    );
    let false_result = evaluate_conditional_branch(
        when_false,
        when_false_span.or(fallback_span),
        expected_type,
        expected_diagnostic,
        false_symbols,
        ctx,
    );

    let true_branch_type = known_branch_type(&true_result);
    let false_branch_type = known_branch_type(&false_result);

    // `cond ? anyValue : maybeString` is `any` in tsc — a branch typed `any`
    // absorbs the union.
    if matches!(true_branch_type, Some(Type::Any)) || matches!(false_branch_type, Some(Type::Any)) {
        return InferredExpression::Known(Type::Any);
    }

    if matches!(condition_result, InferredExpression::Unknown) {
        return InferredExpression::Unknown;
    }

    match (true_branch_type, false_branch_type) {
        (Some(true_type), Some(false_type)) => {
            if is_assignable_to(true_type, expected_type) && is_assignable_to(false_type, expected_type)
            {
                InferredExpression::Known(with_type_copy_reason(TypeCopyReason::ExpectedType, || {
                    expected_type.clone()
                }))
            } else {
                InferredExpression::Known(surge_ts_types::union_type(vec![
                    true_type.clone(),
                    false_type.clone(),
                ]))
            }
        }
        _ => InferredExpression::Unknown,
    }
}

/// Evaluates a value checked against a function's return type. A conditional
/// is split so each branch is related, and reported, on its own.
pub(crate) fn evaluate_return_expression_with_expected_type(
    expression: &ParsedExpression,
    fallback_span: Option<SyntaxTextSpan>,
    statement_span: Option<SyntaxTextSpan>,
    return_type: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    ctx.split_returned_conditional = matches!(expression, ParsedExpression::Conditional { .. });
    let result = evaluate_expression_with_expected_type_anchored(
        expression,
        fallback_span,
        statement_span,
        Some(return_type),
        ExpectedTypeDiagnostic::TypeNotAssignable,
        symbols,
        ctx,
    );
    ctx.split_returned_conditional = false;
    result
}

fn check_returned_conditional_branch(
    branch: &ParsedExpression,
    branch_span: Option<SyntaxTextSpan>,
    expected_type: &Type,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    ctx.split_returned_conditional = matches!(branch, ParsedExpression::Conditional { .. });
    let result = evaluate_expression_with_expected_type(
        branch,
        branch_span,
        Some(expected_type),
        expected_diagnostic,
        symbols,
        ctx,
    );
    ctx.split_returned_conditional = false;
    match result {
        InferredExpression::Known(branch_type) => {
            // tsc's `checkReturnExpression` awaits each branch of an async
            // function's returned conditional before relating it.
            let branch_type = if ctx.in_async_body {
                crate::checks::call::awaited_type(&branch_type)
            } else {
                branch_type
            };
            if branch_type.is_unmodelled() || is_assignable_to(&branch_type, expected_type) {
                return false;
            }
            push_expected_type_mismatch(&branch_type, expected_type, branch_span, expected_diagnostic, ctx);
            true
        }
        // A nested conditional reports its own branches and yields the sentinel.
        _ => true,
    }
}

/// A branch evaluated under the conditional's contextual type. When the branch
/// does not fit that type, the mismatches elaborated inside it are dropped and
/// the branch is typed on its own, so only the whole conditional is reported.
fn evaluate_conditional_branch(
    branch: &ParsedExpression,
    branch_span: Option<SyntaxTextSpan>,
    expected_type: &Type,
    expected_diagnostic: ExpectedTypeDiagnostic,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> InferredExpression {
    // An arrow branch is typed by the contextual signature and keeps the
    // return its body gives: the union of the branches is what gets related.
    if matches!(branch, ParsedExpression::ArrowFunction(_)) {
        crate::checks::function::NEXT_ARROW_NOT_ELABORATED.set(true);
        let contextual = evaluate_expression_with_expected_type(
            branch,
            branch_span,
            Some(expected_type),
            expected_diagnostic,
            symbols,
            ctx,
        );
        crate::checks::function::NEXT_ARROW_NOT_ELABORATED.set(false);
        return contextual;
    }
    let checkpoint = ctx.diagnostics().len();
    let contextual = evaluate_expression_with_expected_type(
        branch,
        branch_span,
        Some(expected_type),
        expected_diagnostic,
        symbols,
        ctx,
    );
    let fits = match &contextual {
        InferredExpression::Known(ty) => ty.is_unknown() || is_assignable_to(ty, expected_type),
        _ => ctx.diagnostics().len() == checkpoint,
    };
    if fits {
        return contextual;
    }
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    evaluate_expression(branch, branch_span, symbols, ctx)
}

fn known_branch_type(branch_result: &InferredExpression) -> Option<&Type> {
    match branch_result {
        InferredExpression::Known(ty) if !ty.is_unknown() => Some(ty),
        _ => None,
    }
}

/// The operand a (possibly nested) `satisfies` relates, with its span, when it
/// is a literal or a name: what the check sees through it carries no
/// diagnostics of its own to repeat.
fn satisfies_operand(expression: &ParsedExpression) -> Option<(&ParsedExpression, Option<SyntaxTextSpan>)> {
    let ParsedExpression::SatisfiesExpression { .. } = expression else {
        return None;
    };
    let mut current = expression;
    let mut current_span = None;
    while let ParsedExpression::SatisfiesExpression { expression: inner, span, keyword_span, .. } = current {
        // The operand ends before the keyword; a literal keeps no span of its
        // own, so this is where it is found.
        current_span = match (span, keyword_span) {
            (Some(span), Some(keyword)) if keyword.start > span.start + 1 => {
                Some(SyntaxTextSpan { start: span.start, end: keyword.start - 1 })
            }
            _ => None,
        };
        current = inner;
    }
    let own_span = match current {
        ParsedExpression::Identifier { span, .. }
        | ParsedExpression::ObjectLiteral { span, .. }
        | ParsedExpression::ArrayLiteral { span, .. } => *span,
        ParsedExpression::StringLiteral(_)
        | ParsedExpression::NumberLiteral(_)
        | ParsedExpression::BigIntLiteral(_)
        | ParsedExpression::BooleanLiteral(_)
        | ParsedExpression::NullLiteral
        | ParsedExpression::UndefinedLiteral => current_span,
        _ => return None,
    };
    Some((current, own_span.or(current_span)))
}

fn push_expected_type_mismatch(
    source_type: &Type,
    expected_type: &Type,
    span: Option<SyntaxTextSpan>,
    diagnostic_kind: ExpectedTypeDiagnostic,
    ctx: &mut CheckerContext,
) {
    if diagnostic_kind == ExpectedTypeDiagnostic::ContextOnly {
        return;
    }
    // An instantiation over an open argument (`Mock<T>` with `T` bare) is not
    // settled enough to reject; the argument path already declines it.
    if crate::checks::call::is_open_instantiation(source_type)
        && !crate::checks::call::definitely_mismatches(source_type, expected_type)
    {
        return;
    }
    let reported_target = crate::checks::expr::reported_relation_target(source_type, expected_type);
    let (source_type_name, expected_type_name) = crate::checks::expr::disambiguated_pair(
        source_type,
        source_display_name(source_type, &reported_target),
        &reported_target,
        reported_target.name(),
        &ctx.file_name,
    );
    let diagnostic = match diagnostic_kind {
        ExpectedTypeDiagnostic::TypeNotAssignable | ExpectedTypeDiagnostic::ContextOnly => {
            crate::checks::expr::type_not_assignable_diagnostic(
                source_type,
                &reported_target,
                &source_type_name,
                &expected_type_name,
                ctx.file_name.clone(),
            )
        }
        ExpectedTypeDiagnostic::ArgumentNotAssignable => {
            crate::checks::expr::argument_not_assignable_diagnostic(
                source_type,
                &reported_target,
                &source_type_name,
                &expected_type_name,
                ctx.file_name.clone(),
            )
        }
        ExpectedTypeDiagnostic::SatisfiesNotAssignable => {
            crate::checks::expr::satisfies_mismatch_diagnostic(
                source_type,
                &reported_target,
                &source_type_name,
                &expected_type_name,
                ctx.file_name.clone(),
            )
        }
    };

    ctx.push(diagnostic_with_syntax_span(diagnostic, span));
}

fn is_global_function_type(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    let Some((file, "Function")) = reference.id.split_once('\u{0}') else {
        return false;
    };
    reference.arguments.is_empty()
        && (crate::default_lib::is_physical_default_lib_file_name(file)
            || crate::default_lib::is_generated_default_lib_file_name(file))
}
