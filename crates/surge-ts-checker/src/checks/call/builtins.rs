//! Built-in call shapes: Array.map/find, Promise.all.

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::{ParsedCallArgument, ParsedExpression, TextSpan as SyntaxTextSpan};
use surge_ts_types::{Type, TypeCopyReason, union_type, with_type_copy_reason};

use crate::checks::expected::{ExpectedTypeDiagnostic, evaluate_expression_with_expected_type};
use crate::context::CheckerContext;
use crate::infer::InferredExpression;
use crate::metrics::alloc_function_type;
use crate::spans::diagnostic_with_syntax_span;
use crate::symbols::SymbolTable;

/// The `(element, index, array)` parameter list for an array iteration
/// callback. Supplying all three (rather than just the element) lets a
/// `(v, i) => …` callback contextually type its index as `number` instead of
/// leaving it implicitly `any` (`TS7006`).
fn array_iteration_callback_parameters(element_type: &Type) -> Vec<Type> {
    let element = with_type_copy_reason(TypeCopyReason::PropertyCallResolution, || {
        element_type.clone()
    });
    let array = with_type_copy_reason(TypeCopyReason::PropertyCallResolution, || {
        Type::Array(Box::new(element_type.clone()))
    });
    vec![element, Type::Number, array]
}

pub(crate) fn check_array_map_call(
    element_type: &Type,
    property_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if arguments.is_empty() {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2554(1, 0, ctx.file_name.clone()),
            call_span.or(property_span),
        ));
        return None;
    }

    let callback_type = Type::Function(alloc_function_type(
        array_iteration_callback_parameters(element_type),
        Type::Any,
        false,
        // `(value, index, array)` are all required in the lib's signature.
        3,
    ));

    let inferred_callback = evaluate_expression_with_expected_type(
        &arguments[0].expression,
        arguments[0].span,
        Some(&callback_type),
        ExpectedTypeDiagnostic::ArgumentNotAssignable,
        symbols,
        ctx,
    );
    let inferred_callback =
        tuple_shaped_callback(&arguments[0], &callback_type, expected_return_type, symbols, ctx)
            .unwrap_or(inferred_callback);

    match inferred_callback {
        InferredExpression::Known(Type::Function(function_type)) => Some(Type::Array(Box::new(
            with_type_copy_reason(TypeCopyReason::PropertyCallResolution, || {
                function_type.return_type().clone()
            }),
        ))),
        InferredExpression::Known(Type::Any) => Some(Type::Array(Box::new(Type::Any))),
        InferredExpression::Known(Type::Unknown)
        | InferredExpression::Known(Type::GenuineUnknown) => None,
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. }
        | InferredExpression::Unknown => None,
        // A callable value (`xs.map(Number)`) maps to what its call
        // signature returns; anything else is no callback surge can read.
        InferredExpression::Known(other) => match other.peeled() {
            Type::Function(function_type) => Some(Type::Array(Box::new(function_type.return_type().clone()))),
            Type::Object(object) => object
                .call_signature()
                .filter(|signature| signature.overloads().is_none())
                .map(|signature| Type::Array(Box::new(signature.return_type().clone()))),
            _ => None,
        },
    }
}

/// `flat<A, D extends number = 1>(this: A, depth?: D): FlatArray<A, D>[]` for
/// a depth written as a literal (or left at its default): each level unwraps
/// the arrays and tuples among the elements. `None` for any other depth.
pub(crate) fn array_flat_result(element_type: &Type, arguments: &[ParsedCallArgument]) -> Option<Type> {
    let depth = match arguments {
        [] => 1,
        [argument] if !argument.spread => match &argument.expression {
            ParsedExpression::NumberLiteral(value) => value.parse::<u32>().ok()?,
            _ => return None,
        },
        _ => return None,
    };
    fn flattened(element: &Type, depth: u32) -> Option<Type> {
        if depth == 0 {
            return Some(element.clone());
        }
        match element {
            Type::Union(union) => union
                .types()
                .iter()
                .map(|member| flattened(member, depth))
                .collect::<Option<Vec<_>>>()
                .map(union_type),
            Type::Array(inner) => flattened(inner, depth - 1),
            Type::Tuple(items) => flattened(&union_type(items.to_vec()), depth - 1),
            Type::Reference(reference) if reference.is_readonly_array() => flattened(&reference.resolve(), depth),
            other if other.is_unknown() => None,
            other => Some(other.clone()),
        }
    }
    Some(Type::Array(Box::new(flattened(element_type, depth)?)))
}

/// `flatMap<U>(callback: (value: T, …) => U | ReadonlyArray<U>): U[]`: the
/// callback is read as `map`'s is, and an array it returns is flattened one
/// level into `U`.
pub(crate) fn check_array_flat_map_call(
    element_type: &Type,
    property_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    fn flattened(ty: &Type) -> Type {
        match ty {
            Type::Array(element) => element.as_ref().clone(),
            Type::Tuple(elements) => union_type(elements.to_vec()),
            Type::Union(union) => union_type(union.types().iter().map(flattened).collect()),
            Type::Reference(reference) if reference.is_readonly_array() => flattened(&reference.resolve()),
            other => other.clone(),
        }
    }
    let Type::Array(returned) = check_array_map_call(element_type, property_span, call_span, arguments, None, symbols, ctx)? else {
        return None;
    };
    Some(Type::Array(Box::new(flattened(&returned))))
}

/// tsc infers `map`'s `U` with the call's contextual element type standing in
/// for the callback's return, so an array literal the callback returns is a
/// tuple of its own elements where that element is one (`checkArrayLiteral`'s
/// `inTupleContext`). The callback was already checked against an open `U`;
/// this pass only reads its return that way, and reports nothing.
fn tuple_shaped_callback(
    callback: &ParsedCallArgument,
    callback_type: &Type,
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<InferredExpression> {
    let ParsedExpression::ArrowFunction(arrow) = &callback.expression else {
        return None;
    };
    // An optional `entries?: readonly (readonly [K, V])[] | null` parameter
    // (`new Map(xs.map(…))`) or an `Iterable<readonly [PropertyKey, T]>` one
    // (`Object.fromEntries`) is a tuple context for the mapped elements too.
    let expected = surge_ts_types::remove_nullish(expected_return_type?);
    let expected_element = match expected.peeled() {
        Type::Array(element) => *element,
        Type::Reference(reference) if reference.is_readonly_array() => reference.arguments.first()?.clone(),
        _ => crate::checks::expr::iteration_types_of_iterable(&expected, crate::checks::expr::IterationUse::sync(false))?
            .yield_type,
    };
    let tuple_like = |ty: &Type| matches!(ty.peeled(), Type::Tuple(_) | Type::OpenTuple(_));
    let in_tuple_context = match expected_element.peeled() {
        Type::Union(union) => union.types().iter().any(tuple_like),
        other => tuple_like(&other),
    };
    if !in_tuple_context {
        return None;
    }
    let mut marked = arrow.clone();
    match &mut marked.body {
        surge_ts_syntax::ParsedArrowFunctionBody::Expression(expression) => mark_tuple_return(expression),
        surge_ts_syntax::ParsedArrowFunctionBody::Block(body) => mark_tuple_returns(body),
    }
    let checkpoint = ctx.diagnostics().len();
    let shaped = evaluate_expression_with_expected_type(
        &ParsedExpression::ArrowFunction(marked),
        callback.span,
        Some(callback_type),
        ExpectedTypeDiagnostic::ContextOnly,
        symbols,
        ctx,
    );
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    match &shaped {
        InferredExpression::Known(Type::Function(function)) if !function.return_type().is_unmodelled() => {
            Some(shaped)
        }
        _ => None,
    }
}

/// Marks an array literal a callback returns as written in a tuple context.
fn mark_tuple_return(expression: &mut ParsedExpression) {
    match expression {
        ParsedExpression::ArrayLiteral { elements, tuple_context, .. }
            if !elements.iter().any(|element| element.spread) =>
        {
            *tuple_context = true;
        }
        ParsedExpression::Conditional { when_true, when_false, .. } => {
            mark_tuple_return(when_true);
            mark_tuple_return(when_false);
        }
        _ => {}
    }
}

fn mark_tuple_returns(statements: &mut [surge_ts_syntax::ParsedFunctionBodyStatement]) {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;
    for statement in statements {
        match statement {
            Statement::Return(statement) => {
                if let Some(expression) = statement.expression.as_mut() {
                    mark_tuple_return(expression);
                }
            }
            Statement::Block(body) => mark_tuple_returns(body),
            Statement::If(statement) => {
                mark_tuple_returns(&mut statement.then_body);
                mark_tuple_returns(&mut statement.else_body);
            }
            Statement::While(statement) => mark_tuple_returns(&mut statement.body),
            Statement::ForOf(statement) => mark_tuple_returns(&mut statement.body),
            Statement::Switch(statement) => {
                for case in &mut statement.cases {
                    mark_tuple_returns(&mut case.consequent);
                }
            }
            Statement::Try(statement) => {
                mark_tuple_returns(&mut statement.block);
                if let Some(handler) = statement.handler.as_mut() {
                    mark_tuple_returns(&mut handler.body);
                }
                mark_tuple_returns(&mut statement.finalizer);
            }
            _ => {}
        }
    }
}

/// `Array.prototype.reduce` / `reduceRight`, by the lib's three overloads:
/// with no initial value the accumulator is the element type; an initial value
/// that fits the element type keeps it (`reduce((a, x) => a + x, 0)`);
/// anything else is the generic overload, whose `U` is the explicit type
/// argument or the initial value's widened type. The callback's return is not
/// compared against `U` here — only its parameters are given their types.
pub(crate) fn check_array_reduce_call(
    element_type: &Type,
    type_arguments: &[surge_ts_syntax::ParsedType],
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Type {
    let (callback, initial) = match arguments {
        [callback, initial] => (callback, Some(initial)),
        [callback, ..] => (callback, None),
        [] => return Type::Any,
    };
    // A type argument or an initial value surge cannot type leaves the
    // accumulator `any`; the callback is still checked against that.
    let explicit = type_arguments
        .first()
        .map(|type_argument| crate::infer::map_parsed_type(type_argument.clone(), ctx))
        .map(|mapped| if mapped.is_unknown() { Type::Any } else { mapped });
    let mut generic_accumulator = false;
    let accumulator = match initial {
        None => explicit.unwrap_or_else(|| element_type.clone()),
        Some(initial) => {
            let evaluated = evaluate_expression_with_expected_type(
                &initial.expression,
                initial.span,
                explicit.as_ref().filter(|explicit| !matches!(explicit, Type::Any)),
                ExpectedTypeDiagnostic::ArgumentNotAssignable,
                symbols,
                ctx,
            );
            match (explicit, evaluated) {
                (Some(explicit), _) => explicit,
                // `reduce(callbackfn, initialValue: T)` takes an initial value
                // the element type admits as it is; any other selects
                // `reduce<U>`, whose inference widens it (`getWidenedType`
                // keeps `undefined` under strict null checks).
                (None, InferredExpression::Known(initial_type)) if !initial_type.is_unknown() => {
                    if surge_ts_types::is_assignable_to(&initial_type, element_type) {
                        element_type.clone()
                    } else if surge_ts_types::strict_null_checks() {
                        generic_accumulator = true;
                        crate::checks::expr::widen_type(&initial_type)
                    } else {
                        generic_accumulator = true;
                        crate::checks::var::widen_implicit_variable_initializer_type(
                            crate::symbols::SymbolKind::Let,
                            &initial.expression,
                            &initial_type,
                            false,
                        )
                    }
                }
                _ => Type::Any,
            }
        }
    };
    let reduce_callback_parameters = |accumulator: &Type| {
        vec![
            accumulator.clone(),
            element_type.clone(),
            Type::Number,
            Type::Array(Box::new(element_type.clone())),
        ]
    };
    let callback_type =
        Type::Function(alloc_function_type(reduce_callback_parameters(&accumulator), Type::Any, false, 4));
    let evaluated = evaluate_expression_with_expected_type(
        &callback.expression,
        callback.span,
        Some(&callback_type),
        ExpectedTypeDiagnostic::ArgumentNotAssignable,
        symbols,
        ctx,
    );
    // A callback whose accumulator is annotated reads no `U` from its context,
    // so its return is an inference candidate beside the initial value
    // (`(previous: undefined | number, …) => number` with `undefined` is
    // `number | undefined`); an unannotated one fixes `U` at the initial value.
    let accumulator_annotated = matches!(
        &callback.expression,
        ParsedExpression::ArrowFunction(arrow) if arrow.parameters.first().is_some_and(|parameter| parameter.declared_type.is_some())
    );
    let accumulator = match &evaluated {
        InferredExpression::Known(Type::Function(callback_function))
            if generic_accumulator && accumulator_annotated && !callback_function.return_type().is_unmodelled() =>
        {
            common_supertype(&accumulator, &crate::checks::expr::widen_type(callback_function.return_type()))
        }
        _ => accumulator,
    };
    let callback_parameters = reduce_callback_parameters(&accumulator);
    let callback_type = Type::Function(alloc_function_type(callback_parameters.clone(), Type::Any, false, 4));
    // The callback relates to the overload's parameter as a whole: a parameter
    // annotation the overload cannot supply (`key: keyof S` for a `string`
    // element) is TS2345, whatever the callback returns.
    if let InferredExpression::Known(Type::Function(callback_function)) = &evaluated {
        let parameters_only = Type::Function(alloc_function_type(
            callback_function.parameters().to_vec(),
            Type::Any,
            callback_function.is_variadic(),
            callback_function.required_parameter_count(),
        ));
        if !surge_ts_types::is_assignable_to(&parameters_only, &callback_type) {
            let source = Type::Function(callback_function.clone());
            let target = Type::Function(
                alloc_function_type(callback_parameters, accumulator.clone(), false, 4).with_parameter_names(vec![
                    Some("previousValue".into()),
                    Some("currentValue".into()),
                    Some("currentIndex".into()),
                    Some("array".into()),
                ]),
            );
            let diagnostic = crate::checks::expr::argument_not_assignable_diagnostic(
                &source,
                &target,
                &source.name(),
                &target.name(),
                ctx.file_name.clone(),
            );
            ctx.push(diagnostic_with_syntax_span(diagnostic, callback.span));
        }
    }
    accumulator
}

/// tsc's `getCommonSupertype` of two inference candidates: the supertype of
/// their non-nullable parts, with the nullable ones added back.
fn common_supertype(first: &Type, second: &Type) -> Type {
    let nullable = |ty: &Type| -> Vec<Type> {
        let members = match ty {
            Type::Union(union) => union.types().to_vec(),
            other => vec![other.clone()],
        };
        members.into_iter().filter(|member| matches!(member, Type::Undefined | Type::Null)).collect()
    };
    let bases: Vec<Type> = [first, second]
        .into_iter()
        .map(surge_ts_types::remove_nullish)
        .filter(|base| !matches!(base, Type::Never))
        .collect();
    let base = bases.into_iter().reduce(|supertype, candidate| {
        if surge_ts_types::is_assignable_to(&supertype, &candidate) { candidate } else { supertype }
    });
    let mut members: Vec<Type> = base.into_iter().collect();
    members.extend(nullable(first));
    members.extend(nullable(second));
    surge_ts_types::union_type(members)
}

pub(crate) fn check_array_find_call(
    element_type: &Type,
    property_span: Option<SyntaxTextSpan>,
    call_span: Option<SyntaxTextSpan>,
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if arguments.is_empty() {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2554(1, 0, ctx.file_name.clone()),
            call_span.or(property_span),
        ));
        return None;
    }

    // The lib's non-narrowing `find` overload types the predicate as returning
    // `unknown`, so any truthy value is accepted.
    let callback_type = Type::Function(alloc_function_type(
        array_iteration_callback_parameters(element_type),
        Type::Any,
        false,
        // `(value, index, array)` are all required in the lib's signature.
        3,
    ));

    let inferred_callback = evaluate_expression_with_expected_type(
        &arguments[0].expression,
        arguments[0].span,
        Some(&callback_type),
        ExpectedTypeDiagnostic::ArgumentNotAssignable,
        symbols,
        ctx,
    );

    match inferred_callback {
        InferredExpression::Known(Type::Function(_)) => Some(surge_ts_types::union_type(vec![
            with_type_copy_reason(TypeCopyReason::PropertyCallResolution, || {
                element_type.clone()
            }),
            Type::Undefined,
        ])),
        InferredExpression::Known(Type::Any) => Some(Type::Any),
        InferredExpression::Known(Type::Unknown)
        | InferredExpression::Known(Type::GenuineUnknown) => None,
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. }
        | InferredExpression::Unknown => None,
        InferredExpression::Known(other) => Some(other),
    }
}

pub(crate) fn is_promise_all_receiver(object_type: &Type) -> bool {
    // The lib's `PromiseConstructor` is usually still a lazy reference here.
    match object_type.peeled() {
        // Declared members only: an index signature answers any name, and a
        // drizzle query read as `PromiseConstructor` had its `.all()` checked
        // as `Promise.all()` (TS2554).
        // A named type is the lib's constructor only by that name: a class
        // with static `resolve` and `all` (`typeof LoomPromise`) is its own.
        Type::Object(object) => {
            object.alias_name.as_deref().is_none_or(|name| name == "PromiseConstructor")
                && object.properties.contains_key("resolve")
                && object.properties.contains_key("all")
        }
        _ => false,
    }
}

/// `Promise.resolve(value)`: a promise of what `value` awaits to, and
/// `Promise<void>` with no argument. The lib's `Promise<Awaited<T>>` awaits
/// each member of a union argument (`T | PromiseLike<T>` resolves to a
/// `Promise<T>`).
pub(crate) fn check_promise_resolve_call(
    arguments: &[ParsedCallArgument],
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let Some(argument) = arguments.first() else {
        return Some(super::promise_of(&Type::Void, ctx));
    };
    // `T` is inferred from the argument with the contextual return type as a
    // second source, so under `Promise<"data">` the literal `"data"` stays the
    // literal it is; with no such context it widens like any inferred `T`.
    let contextual = expected_return_type
        .map(super::awaited_type)
        .filter(|contextual| !contextual.is_unknown() && !matches!(contextual, Type::Any));
    let checkpoint = ctx.diagnostics().len();
    let evaluated = evaluate_expression_with_expected_type(
        &argument.expression,
        argument.span,
        contextual.as_ref(),
        ExpectedTypeDiagnostic::ArgumentNotAssignable,
        symbols,
        ctx,
    );
    match evaluated {
        InferredExpression::Known(ty) if !ty.is_unknown() => {
            // A literal stays one only where the context is literal-like for
            // it (`isLiteralOfContextualType`): `Promise<number>` widens `1`.
            let literal = matches!(ty, Type::StringLiteral(_) | Type::NumberLiteral(_) | Type::BooleanLiteral(_));
            let fits_context = contextual.as_ref().is_some_and(|contextual| {
                surge_ts_types::is_assignable_to(&ty, contextual)
                    && (!literal || crate::checks::function::is_literal_of_contextual_type(&ty, contextual))
            });
            let value = if fits_context {
                ty
            } else {
                crate::checks::var::widen_implicit_variable_initializer_type(
                    crate::symbols::SymbolKind::Let,
                    &argument.expression,
                    &ty,
                    false,
                )
            };
            Some(super::promise_of(&super::awaited_type(&value), ctx))
        }
        // A value that does not fit the context is the enclosing relation's to
        // report, against the promise — not this argument's against `T`.
        _ => {
            if contextual.is_some() {
                ctx.truncate_diagnostics(checkpoint);
            }
            None
        }
    }
}

/// `Promise.race([a, b])`: a promise of what any one element awaits to — the
/// union the lib's `Awaited<T[number]>` names over an array-literal argument.
pub(crate) fn check_promise_race_call(
    arguments: &[ParsedCallArgument],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let [argument] = arguments else {
        return None;
    };
    let ParsedExpression::ArrayLiteral { elements, .. } = &argument.expression else {
        return None;
    };
    if elements.is_empty() || elements.iter().any(|element| element.spread) {
        return None;
    }
    let _ = evaluate_expression_with_expected_type(
        &argument.expression,
        argument.span,
        None,
        ExpectedTypeDiagnostic::ArgumentNotAssignable,
        symbols,
        ctx,
    );
    let reported = ctx.diagnostics().len();
    let mut awaited = Vec::with_capacity(elements.len());
    for element in elements {
        match crate::infer::infer_expression(&element.expression, symbols, ctx) {
            // An element's fresh literal widens as an inferred `T` does
            // (`[p, 1]` races to `string | number`); `as const` keeps it.
            InferredExpression::Known(ty) if !ty.is_unknown() => {
                awaited.push(super::awaited_type(
                    &crate::checks::var::widen_implicit_variable_initializer_type(
                        crate::symbols::SymbolKind::Let,
                        &element.expression,
                        &ty,
                        false,
                    ),
                ));
            }
            _ => {
                ctx.truncate_diagnostics(reported);
                return None;
            }
        }
    }
    ctx.truncate_diagnostics(reported);
    Some(super::promise_of(&surge_ts_types::union_type(awaited), ctx))
}

pub(crate) fn check_promise_all_call(
    arguments: &[ParsedCallArgument],
    call_span: Option<SyntaxTextSpan>,
    expected_return_type: Option<&Type>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let result = promise_all_result(arguments, call_span, symbols, ctx)?;
    // The contextual return type also types the argument (an element written
    // `[a, b]` is a tuple under `Promise<[A, B][]>`), which this path does not
    // do; a result that misses the context is therefore not asserted.
    let contextual = expected_return_type
        .map(super::awaited_type)
        .filter(|contextual| !contextual.is_unknown() && !matches!(contextual, Type::Any));
    if let Some(contextual) = contextual
        && !surge_ts_types::is_assignable_to(&super::awaited_type(&result), &contextual)
    {
        return None;
    }
    Some(result)
}

fn promise_all_result(
    arguments: &[ParsedCallArgument],
    call_span: Option<SyntaxTextSpan>,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if arguments.is_empty() {
        ctx.push(diagnostic_with_syntax_span(
            Diagnostic::ts2554(1, 0, ctx.file_name.clone()),
            call_span,
        ));
        return None;
    }

    let inferred = evaluate_expression_with_expected_type(
        &arguments[0].expression,
        arguments[0].span,
        None,
        ExpectedTypeDiagnostic::ArgumentNotAssignable,
        symbols,
        ctx,
    );

    // `Promise.all([a, b])` answers a tuple of what each element awaits to
    // (the lib's `{ -readonly [P in keyof T]: Awaited<T[P]> }` over a tuple
    // `T`), which is what `const [x, y] = await Promise.all([…])` destructures.
    if let ParsedExpression::ArrayLiteral { elements, .. } = &arguments[0].expression
        && !elements.is_empty()
        && !elements.iter().any(|element| element.spread)
        && matches!(inferred, InferredExpression::Known(_))
    {
        let reported = ctx.diagnostics().len();
        // Each element is checked in a mutable location against the
        // constraint's `unknown`, which widens a fresh literal (`[p, 1]` is
        // `[Promise<string>, number]`); the literal is typed as that tuple.
        let written: Vec<Type> = elements
            .iter()
            .map(|element| match crate::infer::infer_expression(&element.expression, symbols, ctx) {
                InferredExpression::Known(ty) if crate::checks::function::returns_fresh_literal(&element.expression, symbols, ctx) => {
                    match ty {
                        Type::Union(union) => surge_ts_types::union_type(
                            union.types().iter().map(|member| crate::checks::function::widen_unit_return_type(member.clone(), None)).collect(),
                        ),
                        ty => crate::checks::function::widen_unit_return_type(ty, None),
                    }
                }
                InferredExpression::Known(ty) => ty,
                _ => Type::Any,
            })
            .collect();
        ctx.truncate_diagnostics(reported);
        let awaited: Vec<Type> = written.iter().map(super::awaited_type).collect();
        crate::semantic::record_expression_type(&arguments[0].expression, &Type::tuple(written), ctx);
        return Some(super::promise_of(&Type::tuple(awaited), ctx));
    }

    let inferred = match inferred {
        InferredExpression::Known(ty) => InferredExpression::Known(match ty {
            Type::Array(element) => Type::Array(Box::new(super::awaited_type(&element))),
            other => other,
        }),
        other => other,
    };
    let all = match inferred {
        InferredExpression::Known(Type::Array(element_type)) => Some(Type::Array(Box::new(
            with_type_copy_reason(TypeCopyReason::PropertyCallResolution, || {
                (*element_type).clone()
            }),
        ))),
        InferredExpression::Known(Type::Tuple(elements)) => {
            Some(Type::Array(Box::new(if elements.is_empty() {
                Type::Any
            } else {
                surge_ts_types::union_type(elements.into_elements())
            })))
        }
        InferredExpression::Known(Type::Any) => Some(Type::Array(Box::new(Type::Any))),
        // Any other iterable (a readonly tuple, a `Set`) has a shape this path
        // does not map; making one up would be asserted downstream.
        InferredExpression::Known(_) => None,
        InferredExpression::UnresolvedIdentifier { .. }
        | InferredExpression::MissingProperty { .. }
        | InferredExpression::Unknown => Some(Type::Array(Box::new(Type::Any))),
    };
    all.map(|all| super::promise_of(&all, ctx))
}
