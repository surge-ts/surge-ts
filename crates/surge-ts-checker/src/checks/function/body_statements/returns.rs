
use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{
    ParsedExpression, ParsedReturnStatement,
};
use surge_ts_types::{Type, is_assignable_to};

use crate::checks::expected::evaluate_return_expression_with_expected_type;
use crate::checks::expr::evaluate_expression;
use crate::context::CheckerContext;
use crate::context::convert_span;
use crate::flow::{
    FlowCheck, FunctionFlowState, check_expression_flow,
};
use crate::infer::InferredExpression;
use crate::symbols::SymbolTable;

/// Whether a return value could possibly infer as `any`, so the probe below is
/// worth running. A literal, an object/array literal, an arrow or a JSX element
/// has a shape of its own and never is.
///
/// This is not only an optimization. The inference pass is diagnostic-free for
/// every kind EXCEPT an object literal: `infer_object_property_type` routes
/// method and accessor shorthand through the *checking* entry — deliberately, to
/// honor its declared signature — with no expected type, so probing an object
/// literal reported its methods' parameters as implicit any while the real
/// check, one pass later, typed them correctly from the contextual signature.
pub(super) fn may_infer_as_any(expression: &ParsedExpression) -> bool {
    !matches!(
        expression,
        ParsedExpression::ObjectLiteral { .. }
            | ParsedExpression::ArrayLiteral { .. }
            | ParsedExpression::ArrowFunction(_)
            | ParsedExpression::JsxElement { .. }
            | ParsedExpression::JsxFragment { .. }
            | ParsedExpression::StringLiteral(_)
            | ParsedExpression::NumberLiteral(_)
            | ParsedExpression::BigIntLiteral(_)
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::TemplateLiteral { .. }
            | ParsedExpression::UndefinedLiteral
            | ParsedExpression::NullLiteral
    )
}

/// Whether a return value's own type is `any`, directly or as a union member —
/// the shape that collapses tsc's inferred return type. See
/// `ContextualReturnFrame`.
pub(super) fn returns_any(inferred: &InferredExpression) -> bool {
    let InferredExpression::Known(ty) = inferred else {
        return false;
    };
    match ty {
        Type::Any => true,
        Type::Union(union) => union.types().iter().any(|member| *member == Type::Any),
        _ => false,
    }
}

thread_local! {
    static IN_CONSTRUCTOR_BODY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the function body being checked as a class constructor's, or not,
/// until dropped. Every function body enters one, so a nested function never
/// inherits its enclosing constructor's.
pub(crate) struct ConstructorBody(bool);

impl ConstructorBody {
    pub(crate) fn enter(is_constructor: bool) -> Self {
        Self(IN_CONSTRUCTOR_BODY.with(|current| current.replace(is_constructor)))
    }
}

impl Drop for ConstructorBody {
    fn drop(&mut self) {
        let outer = self.0;
        IN_CONSTRUCTOR_BODY.with(|current| current.set(outer));
    }
}

pub(crate) fn check_function_return_statement(
    return_statement: ParsedReturnStatement,
    statement_index: usize,
    return_type: Option<&Type>,
    flow_state: &mut FunctionFlowState,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) {
    // tsc's `checkReturnStatement` relates a constructor's `return` value to
    // the instance type and reports TS2409 beside the mismatch; a bare
    // `return` there relates nothing.
    let in_constructor = IN_CONSTRUCTOR_BODY.with(std::cell::Cell::get);
    let Some(expression) = return_statement.expression.as_ref() else {
        if !in_constructor {
            check_bare_return(return_statement.span, return_type, ctx);
        }
        return;
    };

    let flow_blocked = if flow_state.tracked_local_count() > 0 {
        if return_type.is_some_and(crate::flow::is_non_generic_contextual_type) {
            crate::flow::check_substituting_read_flow(
                expression,
                return_statement.expression_span,
                flow_state,
                statement_index,
                ctx,
            )
        } else {
            check_expression_flow(
                expression,
                return_statement.expression_span,
                flow_state,
                statement_index,
                ctx,
            )
        }
    } else {
        FlowCheck::Clear
    };

    if flow_blocked.is_blocked() {
        return;
    }

    let Some(return_type) = return_type else {
        // No expected return type only removes the assignability verdict; the
        // value still owes its own diagnostics. Skipping it left every
        // expression in an unannotated function unchecked — unresolved names,
        // UMD globals behind JSX tags, property access — which is why a
        // component written as `const C = () => { return <div/>; }` reported
        // nothing at all.
        // …but with no expectation there is also no contextual parameter type
        // to hand a callback or an object-literal method, so an implicit-any
        // report here would describe surge's missing context rather than an
        // omission in the source (tRPC's `new ReadableStream({ start(c) {…} })`
        // inside a returned object is typed by the constructor, not by the
        // return). Same reasoning as a degraded expectation.
        //
        // Not for an unannotated declaration, though: there the missing
        // expectation is the source's, exactly as in Go, and suppressing it hid
        // real implicit-`any` callbacks in returned JSX alongside surge's gaps.
        let suppress = !ctx.in_unannotated_declaration_body();
        if suppress {
            ctx.degraded_expected_type_depth += 1;
        }
        let inferred = evaluate_expression(
            expression,
            return_statement.expression_span,
            symbols,
            ctx,
        );
        if suppress {
            ctx.degraded_expected_type_depth -= 1;
        }
        match inferred {
            InferredExpression::Known(source_type) => ctx.note_contextual_return_type(&source_type, Some(expression), symbols),
            // A value surge could not type still counts as returned: tsc knows
            // its type and decides TS7030 from it, so the sentinel must suppress
            // the report the same way a known `unknown` result does.
            _ => ctx.note_contextual_return_type(&Type::Unknown, Some(expression), symbols),
        }
        return;
    };

    // tsc's `unwrapReturnType`: a generator returns the return type argument of
    // the generator type it is declared with
    // (`getIterationTypeOfGeneratorFunctionReturnType`); a type that is not one
    // leaves the value unchecked.
    let generator_return_type;
    let return_type = if ctx.in_generator_body {
        let Some(declared) = generator_return_target(return_type, ctx) else {
            let _ = evaluate_expression(expression, return_statement.expression_span, symbols, ctx);
            return;
        };
        generator_return_type = declared;
        &generator_return_type
    } else {
        return_type
    };

    // tsc's `unwrapReturnType`: what an async function returns is related — and
    // contextually typed — by the awaited return type, so `return { … }` under
    // `Promise<R>` is read against `R`.
    let awaited_return_type;
    let return_type = if ctx.in_async_body {
        awaited_return_type = crate::checks::call::awaited_type(return_type);
        &awaited_return_type
    } else {
        return_type
    };

    // Only the mismatch verdicts raised while checking this value belong to the
    // contextual-return frame; everything else the expression reports is
    // unrelated and must survive.
    let was_in_return_check = ctx.in_contextual_return_check;
    ctx.in_contextual_return_check = ctx.in_contextual_return_body();
    let was_unfresh = std::mem::replace(&mut ctx.relating_unfresh_return, ctx.in_contextual_return_check);
    let checkpoint = ctx.diagnostics().len();
    // A constructor relates the whole value (`checkTypeAssignableToAndOptionallyElaborate`):
    // a returned conditional is not split into its branches.
    let inferred_expression = if in_constructor {
        crate::checks::expected::evaluate_expression_with_expected_type_anchored(
            expression,
            return_statement.expression_span,
            return_statement.span,
            Some(return_type),
            crate::checks::expected::ExpectedTypeDiagnostic::TypeNotAssignable,
            symbols,
            ctx,
        )
    } else if ctx.in_async_body {
        crate::checks::call::with_awaited_context(return_type, || {
            evaluate_return_expression_with_expected_type(
                expression,
                return_statement.expression_span,
                return_statement.span,
                return_type,
                symbols,
                ctx,
            )
        })
    } else {
        evaluate_return_expression_with_expected_type(
            expression,
            return_statement.expression_span,
            return_statement.span,
            return_type,
            symbols,
            ctx,
        )
    };
    ctx.in_contextual_return_check = was_in_return_check;
    ctx.relating_unfresh_return = was_unfresh;

    // An `any` return is what collapses tsc's inferred union, so the frame drops
    // its recorded verdicts when the body ends. The contextual evaluation above
    // cannot answer this — it reports per branch and yields the sentinel on a
    // mismatch — so ask the diagnostic-free inference path for the value's own
    // type, which for `cond ? anyValue : { … }` is the union tsc would form.
    if ctx.in_contextual_return_body() && may_infer_as_any(expression) {
        // Diagnostic-free: the value was already checked above, and a type
        // query in it (`as Out<typeof schema>`) is resolved here without the
        // arrow's own parameters in scope.
        let reported = ctx.diagnostics().len();
        let inferred = crate::infer::infer_expression(expression, symbols, ctx);
        ctx.truncate_diagnostics(reported);
        if returns_any(&inferred) {
            ctx.note_contextual_return_is_any();
        }
    }

    match inferred_expression {
        InferredExpression::Known(source_type) => {
            // tsc's `unwrapReturnType`: an async function relates the awaited
            // value to the awaited return type, and that is what its body is
            // taken to return (`return this.p` of a `Promise<void>` is a
            // void-like return under `noImplicitReturns`).
            let unwrapped_return_type;
            let (source_type, return_type) = if ctx.in_async_body {
                unwrapped_return_type = crate::checks::call::awaited_type(return_type);
                (crate::checks::call::awaited_type(&source_type), &unwrapped_return_type)
            } else {
                (source_type, return_type)
            };
            ctx.note_contextual_return_type(&source_type, Some(expression), symbols);
            // A sentinel anywhere in either side means surge lost part of the
            // shape, so a mismatch reflects the modelling gap rather than the
            // source — the same deep guard the variable-declaration check
            // applies.
            if source_type.is_unmodelled() {
                return;
            }

            if !is_assignable_to(&source_type, &return_type) {
                // A sentinel anywhere in either side means surge lost part of
                // the shape, so the mismatch reflects the modelling gap rather
                // than the source — the same deep guard the variable
                // declaration check applies. Checked only after assignability
                // already failed: the deep walk forces lazy references, and
                // doing that on every clean return measurably perturbs later
                // resolutions (a false TS2554 on ky's `resolve()`).
                // A written `unknown` (a method's `(o: unknown)`) is a real type,
                // so only the degradation sentinel counts.
                if (crate::checks::call::as_source(|| {
                    crate::checks::function::type_contains_degradation(&source_type)
                })
                    || crate::checks::function::type_contains_degradation(&return_type))
                    && !crate::checks::assign::definite_primitive_member_mismatch(&source_type, &return_type)
                {
                    return;
                }
                let reported_target =
                    crate::checks::expr::reported_relation_target(&source_type, &return_type);
                let source_type_name =
                    crate::checks::expr::source_display_name(&source_type, &reported_target);
                let target_type_name = reported_target.name();
                let diagnostic = crate::checks::expr::type_not_assignable_diagnostic(
                    &source_type,
                    &reported_target,
                    &source_type_name,
                    &target_type_name,
                    ctx.file_name.clone(),
                );

                let diagnostic = match return_statement.span.or(return_statement.expression_span) {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                };

                // Same frame bookkeeping as the verdicts raised inside the
                // expression above: this is the whole-value one.
                let was_in_return_check = ctx.in_contextual_return_check;
                ctx.in_contextual_return_check = ctx.in_contextual_return_body();
                ctx.push(diagnostic);
                ctx.in_contextual_return_check = was_in_return_check;
                if in_constructor {
                    push_constructor_return_error(return_statement.span, ctx);
                }
            }
        }
        // The expected-type evaluation collapses to the sentinel once it has
        // reported a leaf mismatch, so the return's own type has to come from the
        // diagnostic-free path — it is what renders the whole signature tsc names
        // on the assignment. Only reached on a mismatch inside a contextually
        // typed body, so this costs nothing on a clean return.
        // A failed lookup returns tsc's error type.
        failed @ (InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. }) => {
            ctx.note_contextual_return_type(&failed.flowing_type().unwrap_or(Type::Unknown), Some(expression), symbols);
        }
        InferredExpression::Unknown => {
            // The contextual evaluation yields the sentinel once it has
            // reported a mismatch inside the value, which the whole value
            // then fails too.
            if in_constructor
                && ctx.diagnostics().len() > checkpoint
                && constructor_return_mismatch(expression, return_type, symbols, ctx)
            {
                push_constructor_return_error(return_statement.span, ctx);
            }
            let mut noted = false;
            if ctx.in_contextual_return_body()
                && let InferredExpression::Known(source_type) =
                    crate::infer::infer_expression(expression, symbols, ctx)
            {
                ctx.note_contextual_return_type(&source_type, Some(expression), symbols);
                noted = true;
            }
            if !noted {
                ctx.note_contextual_return_type(&Type::Unknown, Some(expression), symbols);
            }
        }
    }
}

/// Whether a constructor's returned value, typed on its own, is not assignable
/// to the class instance type — with nothing unmodelled on either side.
fn constructor_return_mismatch(
    expression: &ParsedExpression,
    instance_type: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> bool {
    let reported = ctx.diagnostics().len();
    let inferred = crate::infer::infer_expression(expression, symbols, ctx);
    ctx.truncate_diagnostics(reported);
    let InferredExpression::Known(source_type) = inferred else {
        return false;
    };
    !source_type.is_unmodelled()
        && !crate::checks::call::as_source(|| {
            crate::checks::function::type_contains_degradation(&source_type)
        })
        && !crate::checks::function::type_contains_degradation(instance_type)
        && !is_assignable_to(&source_type, instance_type)
}

fn push_constructor_return_error(
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    let diagnostic = Diagnostic::ts2409(ctx.file_name.clone());
    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

/// tsc's `checkReturnStatement` on a `return;`: the value is `undefined`,
/// related to the unwrapped annotation under `strictNullChecks` and to a `never`
/// annotation without it. An unannotated or contextually typed body has no
/// annotation to relate it to.
fn check_bare_return(
    span: Option<surge_ts_syntax::TextSpan>,
    return_type: Option<&Type>,
    ctx: &mut CheckerContext,
) {
    let Some(return_type) = return_type else {
        return;
    };
    if ctx.in_contextual_return_body()
        || (!ctx.options.strict_null_checks && !matches!(return_type.peeled(), Type::Never))
    {
        return;
    }
    let target = if ctx.in_generator_body {
        let Some(declared) = generator_return_target(return_type, ctx) else {
            return;
        };
        declared
    } else {
        return_type.clone()
    };
    let target = if ctx.in_async_body {
        crate::checks::call::awaited_type(&target)
    } else {
        target
    };
    if is_assignable_to(&Type::Undefined, &target)
        || crate::checks::function::type_contains_degradation(&target)
    {
        return;
    }
    let reported_target = crate::checks::expr::reported_relation_target(&Type::Undefined, &target);
    let diagnostic = crate::checks::expr::type_not_assignable_diagnostic(
        &Type::Undefined,
        &reported_target,
        "undefined",
        &reported_target.name(),
        ctx.file_name.clone(),
    );
    ctx.push(match span {
        Some(span) => diagnostic.with_span(convert_span(span)),
        None => diagnostic,
    });
}

/// The `TReturn` of a generator's declared `Generator<T, TReturn, TNext>` (or
/// the iterator and iterable types a generator may be declared as), `any` when
/// the argument is left to its default.
/// The `TReturn` a generator body's `return` is related to. An annotation the
/// resolver handed back expanded (a repeated lib instantiation) no longer names
/// its arguments; its iteration protocol still answers
/// (`getIterationTypesOfGeneratorFunctionReturnType`).
fn generator_return_target(declared: &Type, ctx: &CheckerContext) -> Option<Type> {
    generator_return_type_argument(declared).or_else(|| {
        if !matches!(declared.peeled(), Type::Object(_)) {
            return None;
        }
        crate::checks::expr::generator_return_iteration_types(
            declared,
            ctx.in_async_generator,
            ctx.options.strict_builtin_iterator_return,
        )
        .map(|types| types.return_type)
        .filter(|ty| !ty.is_unmodelled())
    })
}

pub(crate) fn generator_return_type_argument(declared: &Type) -> Option<Type> {
    generator_type_argument(declared, 1)
}

/// The next type a generator annotated `declared` is resumed with: the
/// `TNext` argument of the lib iterator type it names.
pub(crate) fn generator_next_type_argument(declared: &Type) -> Option<Type> {
    generator_type_argument(declared, 2)
}

/// The yield type a generator annotated `declared` yields: the first
/// argument of the lib iterator type it names.
pub(crate) fn generator_yield_type_argument(declared: &Type) -> Option<Type> {
    generator_type_argument(declared, 0)
}

fn generator_type_argument(declared: &Type, index: usize) -> Option<Type> {
    let Type::Reference(reference) = declared else {
        return None;
    };
    let name = reference.id.split('\u{0}').next_back()?;
    matches!(
        name,
        "Generator"
            | "AsyncGenerator"
            | "Iterator"
            | "AsyncIterator"
            | "IterableIterator"
            | "AsyncIterableIterator"
            | "Iterable"
            | "AsyncIterable"
            | "IteratorObject"
            | "AsyncIteratorObject"
    )
    .then(|| reference.arguments.get(index).cloned().unwrap_or(Type::Any))
}

/// tsc's `createGeneratorType`: the lib's `Generator<Y, R, N>`, or
/// `AsyncGenerator<Y, R, N>` for an async generator. `None` when the lib does
/// not declare it.
pub(crate) fn generator_type_of(
    yield_type: &Type,
    return_type: &Type,
    next_type: &Type,
    is_async: bool,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let name = if is_async { "AsyncGenerator" } else { "Generator" };
    let arguments = [yield_type, return_type, next_type];
    // Each slot is named after its argument, as `promise_of` names its own:
    // the instantiation renders its written arguments.
    let mut substitution = crate::infer::TypeParameterSubstitution::new();
    for argument in arguments {
        substitution.insert(argument.name(), argument.clone());
    }
    let named = |name: String, type_arguments| {
        surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name,
            span: None,
            type_arguments,
        }))
    };
    let written = named(
        name.to_string(),
        arguments.iter().map(|argument| named(argument.name(), Vec::new())).collect(),
    );
    let reported = ctx.diagnostics().len();
    let generator = crate::infer::map_parsed_type_with_substitution(written, ctx, &substitution);
    ctx.truncate_diagnostics(reported);
    matches!(generator, Type::Reference(_)).then_some(generator)
}

/// tsc's `createGeneratorType` for an unannotated generator: without the
/// lib's `Generator` it falls back on `IterableIterator`, reported when that
/// is missing too.
pub(crate) fn report_missing_generator_fallback(is_async: bool, ctx: &mut CheckerContext) {
    let generator = if is_async { "AsyncGenerator" } else { "Generator" };
    if ctx.ambient_global_type_declarations.get(generator).is_none() {
        report_missing_global_type(if is_async { "AsyncIterableIterator" } else { "IterableIterator" }, ctx);
    }
}

/// tsc's `getGlobalType(name, arity, true)`: a lib global the checked syntax
/// needs, reported once for the program (TS2318).
pub(crate) fn report_missing_global_type(name: &str, ctx: &mut CheckerContext) {
    if ctx.ambient_global_type_declarations.get(name).is_some() {
        return;
    }
    ctx.push(surge_ts_diagnostics::Diagnostic::new(
        surge_ts_diagnostics::DiagnosticCode::TypeScript(2318),
        format!("Cannot find global type '{name}'."),
        String::new(),
    ));
}

/// tsc's `checkSignatureDeclaration` for a generator's annotation: the
/// generator object built from the annotation's own iteration types must be
/// assignable to it, which rejects an annotation no generator can be
/// (`number`, an interface with members a generator lacks).
pub(crate) fn check_generator_instantiation(
    declared: &Type,
    is_async: bool,
    span: Option<surge_ts_syntax::TextSpan>,
    ctx: &mut CheckerContext,
) {
    if matches!(declared, Type::Any | Type::Void)
        || declared.is_unmodelled()
        || crate::checks::assign::type_contains_unknown(declared)
    {
        return;
    }
    let iteration = generator_type_argument(declared, 0).is_none().then(|| {
        crate::checks::expr::generator_return_iteration_types(
            declared,
            is_async,
            ctx.options.strict_builtin_iterator_return,
        )
        .filter(|types| {
            !types.yield_type.is_unmodelled()
                && !types.return_type.is_unmodelled()
                && types.next_type.as_ref().is_none_or(|next| !next.is_unmodelled())
        })
    });
    let (yield_type, return_type, next_type) = match iteration.flatten() {
        Some(types) => (
            types.yield_type,
            types.return_type,
            types.next_type.unwrap_or(Type::GenuineUnknown),
        ),
        None => {
            let yield_type = generator_type_argument(declared, 0).unwrap_or(Type::Any);
            let return_type = generator_type_argument(declared, 1).unwrap_or_else(|| yield_type.clone());
            let next_type = generator_type_argument(declared, 2).unwrap_or(Type::GenuineUnknown);
            (yield_type, return_type, next_type)
        }
    };
    let Some(generator) = generator_type_of(&yield_type, &return_type, &next_type, is_async, ctx) else {
        return;
    };
    if !is_assignable_to(&generator, declared) {
        crate::checks::var::report_assignability_failure(&generator, declared, span, ctx);
    }
}
