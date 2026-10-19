//! Arrow-function expression inference.

use super::*;

use surge_ts_syntax::{ParsedArrowFunction, ParsedArrowFunctionBody};
use surge_ts_types::Type;

use crate::context::CheckerContext;
use crate::metrics::alloc_function_type;
use crate::symbols::SymbolTable;

use crate::infer::InferredExpression;

pub(crate) fn infer_arrow_function(
    arrow_function: &ParsedArrowFunction,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> surge_ts_types::FunctionType {
    infer_arrow_function_with_contextual_parameters(arrow_function, &[], symbols, ctx)
}

thread_local! {
    /// The contextual return type a generic call gives the callback argument it
    /// is inferring from, taken by the first arrow sketched under it so the
    /// arrows nested in its body do not see it.
    static CALLBACK_CONTEXTUAL_RETURN: std::cell::RefCell<Option<Type>> =
        const { std::cell::RefCell::new(None) };
    /// The parameter type a generic call gives an `xs.map(…)` argument it is
    /// inferring from, taken by that map call.
    static MAPPED_ARGUMENT_CONTEXT: std::cell::RefCell<Option<Type>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `infer` with `contextual` as the type the `map` call it infers is
/// passed to: a tuple element there makes the callback's array literal one.
pub(crate) fn with_mapped_argument_context<R>(contextual: Type, infer: impl FnOnce() -> R) -> R {
    let outer = MAPPED_ARGUMENT_CONTEXT.with(|cell| cell.replace(Some(contextual)));
    let result = infer();
    MAPPED_ARGUMENT_CONTEXT.with(|cell| *cell.borrow_mut() = outer);
    result
}

pub(crate) fn take_mapped_argument_context() -> Option<Type> {
    MAPPED_ARGUMENT_CONTEXT.with(|cell| cell.borrow_mut().take())
}

/// Runs `infer` with `contextual` as the contextual return type of the arrow
/// it sketches first.
pub(crate) fn with_callback_contextual_return<R>(contextual: Type, infer: impl FnOnce() -> R) -> R {
    let outer = CALLBACK_CONTEXTUAL_RETURN.with(|cell| cell.replace(Some(contextual)));
    let result = infer();
    CALLBACK_CONTEXTUAL_RETURN.with(|cell| *cell.borrow_mut() = outer);
    result
}

/// [`infer_arrow_function`] with the parameter types the signature the callback
/// is being passed to gives it. A generic call's second inference pass supplies
/// them: an un-annotated parameter sketched as `any` types the body as `any`
/// too, and a type parameter that appears only in the callback's *return*
/// position (`map<U>(f: (v: T) => U): U`) is then bound to `any` — the call's
/// result silences every diagnostic downstream of it.
pub(crate) fn infer_arrow_function_with_contextual_parameters(
    arrow_function: &ParsedArrowFunction,
    contextual_parameters: &[Type],
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> surge_ts_types::FunctionType {
    // This sketch feeds generic-call inference (`fn(impl?: T)` bound from a
    // `vi.fn((n: number) => …)` argument); the authoritative
    // `check_arrow_function_expression` pass follows with the contextual
    // signature and reports the annotations' own diagnostics, so the ones the
    // mapping emits here are discarded. A generic arrow keeps the untyped
    // sketch: its annotations name type parameters no scope here declares.
    let typed_annotations = arrow_function.type_parameters.is_empty();
    let contextual_return = CALLBACK_CONTEXTUAL_RETURN.with(|cell| cell.replace(None));
    let diagnostics_before = ctx.diagnostics().len();
    let parameters = arrow_function
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| match &parameter.declared_type {
            // An annotation that does not resolve stays `any`, which is what
            // every parameter was before annotations were read at all, so it
            // cannot newly disable an inference that used to succeed.
            Some(declared_type) if typed_annotations => {
                let mapped = map_parsed_type(declared_type.clone(), ctx);
                if mapped.is_unmodelled() && mapped != Type::GenuineUnknown {
                    Type::Any
                } else {
                    mapped
                }
            }
            _ => contextual_parameter_type(contextual_parameters, index),
        })
        .collect::<Vec<_>>();
    let declared_return_type = arrow_function.return_type.as_ref().and_then(|ty| {
        if typed_annotations {
            let mapped = map_parsed_type(ty.clone(), ctx);
            (!mapped.is_unmodelled()).then_some(mapped)
        } else {
            primitive_declared_return_type(ty)
        }
    });
    let written_this = arrow_function
        .this_parameter_type
        .as_ref()
        .filter(|_| typed_annotations && !matches!(arrow_function.this_binding, surge_ts_syntax::ParsedThisBinding::Inherited))
        .map(|written| map_parsed_type(written.clone(), ctx))
        .filter(|this_type| !this_type.is_unmodelled());
    // The authoritative pass resolves the same annotations again, so the
    // once-guard of what is discarded here must not withhold its reports.
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);

    // A generator returns a `Generator`/`AsyncGenerator`, not whatever its body
    // completes with. The sketch stays at the sentinel unless every yield can
    // be typed from here, rather than claiming a shape a caller's type
    // parameter would then bind to (`run(async function* () { yield 'a' })`).
    if arrow_function.is_generator && declared_return_type.is_none() {
        let return_type = match &arrow_function.body {
            ParsedArrowFunctionBody::Block(body) if arrow_function.type_parameters.is_empty() => {
                let locals = body_locals(arrow_function, &parameters, symbols);
                sketched_generator_type(body, locals, contextual_return.as_ref(), arrow_function.is_async, ctx)
            }
            _ => None,
        };
        return alloc_function_type(
            parameters,
            return_type.unwrap_or(Type::Unknown),
            false,
            required_parameter_count(arrow_function.parameters.as_slice()),
        )
        .with_parameter_names(crate::checks::function::written_binding_names(&arrow_function.parameters)).with_question_optional_parameters(crate::checks::function::written_question_optional(&arrow_function.parameters))
        .with_this_type(written_this);
    }

    // What the body's returned values are contextually typed by: an async
    // arrow's returns are read against the awaited contextual return.
    let body_contextual = contextual_return.as_ref().map(|contextual| {
        if arrow_function.is_async {
            crate::checks::call::awaited_type(contextual)
        } else {
            contextual.clone()
        }
    });
    let infer_return_type = |ctx: &mut CheckerContext| match &arrow_function.body {
        ParsedArrowFunctionBody::Expression(expression) => {
            declared_return_type.unwrap_or_else(|| {
                let locals = body_locals(arrow_function, &parameters, symbols);
                if let Some(contextual) = body_contextual.as_ref()
                    && let Some(ty) =
                        call_typed_by_contextual_return(expression, contextual, arrow_function.is_async, &locals, ctx)
                {
                    return ty;
                }
                match infer_expression(expression, &locals, ctx).flowing_type() {
                    Some(ty) => match body_contextual
                        .as_ref()
                        .filter(|contextual| !surge_ts_types::type_variable::mentions_type_variable(contextual))
                    {
                        Some(contextual) => widen_literals_for_contextual_return(expression, ty, contextual),
                        None => widen_fresh_literal_return(expression, ty),
                    },
                    None => Type::Unknown,
                }
            })
        }
        ParsedArrowFunctionBody::Block(body) => declared_return_type
            .or_else(|| {
                let locals = body_locals(arrow_function, &parameters, symbols);
                let return_context = body_contextual.as_ref().map(|contextual| (contextual, arrow_function.is_async));
                infer_block_body_return_type(body, locals, return_context, ctx)
            })
            // A body with no `return` anywhere returns `void`, as tsc types it.
            // Leaving it at the degradation sentinel made the sketch unusable for
            // generic inference: `vi.fn((result) => { … })` inferred no `T`, so
            // `Mock<T>` stayed uninstantiated and every use of the mock was a
            // false error.
            //
            // A body that cannot complete at all (`() => { throw new Error(…) }`)
            // returns `never`, which is assignable everywhere; typing it `void`
            // made zustand's throwing storage stub unassignable to `StateStorage`.
            .or_else(|| {
                (!body_contains_return(body)).then(|| {
                    if crate::flow::analyze_function_body_flow(body).guarantees_exit {
                        Type::Never
                    } else {
                        Type::Void
                    }
                })
            })
            .unwrap_or(Type::Unknown),
    };
    // The body sees the arrow's own type parameters wherever it names them
    // (`<U>() => <U[]>null`), as tsc's lexical `resolveName` finds them.
    let return_type = if arrow_function.type_parameters.is_empty() {
        infer_return_type(ctx)
    } else {
        crate::checks::function::with_type_parameter_scope(
            &arrow_function.type_parameters,
            ctx,
            infer_return_type,
        )
    };

    // An async function returns a promise of what its body completes with.
    let return_type = if arrow_function.is_async
        && arrow_function.return_type.is_none()
        && !return_type.is_unmodelled()
    {
        crate::checks::call::promise_of(&return_type, ctx)
    } else {
        return_type
    };

    // A written `...rest: T[]` makes the sketch variadic, as the declared
    // arrow's own type is, so inference pairs its element with each expected
    // parameter from its position on (`applyToParameterTypes`).
    let annotated_rest = typed_annotations
        && arrow_function
            .parameters
            .last()
            .is_some_and(|parameter| parameter.rest && parameter.declared_type.is_some())
        && matches!(parameters.last(), Some(Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_)));
    let required = if annotated_rest {
        required_parameter_count(&arrow_function.parameters[..arrow_function.parameters.len() - 1])
    } else {
        required_parameter_count(arrow_function.parameters.as_slice())
    };
    // `inferFromSignatures` reads a callback's predicate, written or read from
    // its body (`getTypePredicateFromBody`), so the sketch carries it.
    let type_predicate = sketch_type_predicate(arrow_function, typed_annotations, &parameters, &return_type, symbols, ctx);
    with_written_predicate(
        alloc_function_type(parameters, return_type, annotated_rest, required)
            .with_parameter_names(crate::checks::function::written_binding_names(&arrow_function.parameters)).with_question_optional_parameters(crate::checks::function::written_question_optional(&arrow_function.parameters))
            .with_type_parameter_head(crate::checks::function::type_parameter_head(&arrow_function.type_parameters))
            .with_type_predicate(type_predicate)
            .with_this_type(written_this),
        arrow_function,
        ctx,
    )
}

/// The predicate a callback sketch carries: a written `x is T` (resolved as
/// the sketch's annotations are), or, with no return annotation, the one its
/// boolean body draws over a parameter. The inferred target keeps to the
/// members of a union parameter, as the checker's arrow pass does.
fn sketch_type_predicate(
    arrow_function: &ParsedArrowFunction,
    typed_annotations: bool,
    parameters: &[Type],
    return_type: &Type,
    symbols: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<surge_ts_types::TypePredicate> {
    let checkpoint = ctx.diagnostics().len();
    let predicate = match &arrow_function.return_type {
        Some(written @ surge_ts_syntax::ParsedType::Predicate(_)) if typed_annotations => {
            crate::infer::types::written_type_predicate(
                written,
                arrow_function
                    .parameters
                    .iter()
                    .map(|parameter| crate::checks::function::parameter_identifier_name(parameter).map(str::to_string)),
                ctx,
                |ty, ctx| map_parsed_type(ty.clone(), ctx),
            )
        }
        None if typed_annotations
            && !arrow_function.is_async
            && !arrow_function.is_generator
            && matches!(return_type.peeled(), Type::Boolean | Type::BooleanLiteral(_)) =>
        {
            let inferred = match &arrow_function.body {
                ParsedArrowFunctionBody::Expression(expression) => crate::checks::function::infer_predicate_from_body(
                    &arrow_function.parameters,
                    parameters,
                    expression,
                    symbols,
                    ctx,
                ),
                ParsedArrowFunctionBody::Block(statements) => crate::checks::function::infer_predicate_from_function_body(
                    &arrow_function.parameters,
                    parameters,
                    statements,
                    symbols,
                    ctx,
                ),
            };
            inferred
                .filter(|inferred| match parameters.get(inferred.parameter_index).map(Type::peeled) {
                    Some(Type::Union(declared)) => {
                        let members = match inferred.target.peeled() {
                            Type::Union(target) => target.types().to_vec(),
                            target => vec![target],
                        };
                        members.iter().all(|member| declared.types().contains(member))
                    }
                    _ => true,
                })
                .map(|inferred| surge_ts_types::TypePredicate {
                    kind: surge_ts_types::TypePredicateKind::Identifier,
                    parameter_index: Some(inferred.parameter_index),
                    ty: Some(inferred.target),
                    name: None,
                })
        }
        _ => None,
    };
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    predicate
}

/// tsc's `getReturnTypeFromBody` for a generator the sketch cannot check: its
/// yields' operands inferred in the parameters' scope, every name the body
/// declares shadowed and its top-level declarations bound in order. `None`
/// when a yield reads anything that leaves untyped.
fn sketched_generator_type(
    body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    mut locals: SymbolTable,
    contextual_return: Option<&Type>,
    is_async: bool,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    use surge_ts_syntax::ParsedFunctionBodyStatement;

    let operands = crate::checks::function::yield_operands(body)?;
    let mut declared = std::collections::HashSet::new();
    crate::flow::collect_declared_names(body, &mut declared);
    for name in declared {
        let _ = locals.insert(
            name,
            crate::symbols::SymbolInfo {
                ty: Type::Unknown,
                kind: crate::symbols::SymbolKind::Let,
                function_signature: None,
            },
        );
    }
    for statement in body {
        let ParsedFunctionBodyStatement::VariableDeclaration(variable) = statement else {
            continue;
        };
        let (Some(initializer), None, false) =
            (variable.initializer.as_ref(), variable.declared_type.as_ref(), variable.from_binding_pattern)
        else {
            continue;
        };
        let InferredExpression::Known(ty) = infer_expression(initializer, &locals, ctx) else {
            continue;
        };
        let kind = match variable.kind {
            surge_ts_syntax::ParsedVariableKind::Var => crate::symbols::SymbolKind::Var,
            surge_ts_syntax::ParsedVariableKind::Let => crate::symbols::SymbolKind::Let,
            surge_ts_syntax::ParsedVariableKind::Const => crate::symbols::SymbolKind::Const,
        };
        let ty = crate::checks::var::widen_implicit_variable_initializer_type(kind, initializer, &ty, false);
        let _ = locals.insert(
            variable.name.clone(),
            crate::symbols::SymbolInfo {
                ty,
                kind,
                function_signature: None,
            },
        );
    }
    let mut yields = crate::checks::function::GeneratorYields::default();
    for (operand, delegate) in operands {
        let Some(operand) = operand else {
            if !yields.yielded.contains(&Type::Undefined) {
                yields.yielded.push(Type::Undefined);
            }
            continue;
        };
        let InferredExpression::Known(operand_type) = infer_expression(operand, &locals, ctx) else {
            return None;
        };
        if operand_type.is_unmodelled() {
            return None;
        }
        let yielded = if delegate {
            crate::checks::expr::iterated_element_type(&operand_type)?
        } else if matches!(
            operand,
            surge_ts_syntax::ParsedExpression::ObjectLiteral { .. } | surge_ts_syntax::ParsedExpression::ArrayLiteral { .. }
        ) {
            crate::checks::expr::widen_type(&operand_type)
        } else {
            operand_type.clone()
        };
        let yielded = if is_async { crate::checks::call::awaited_type(&yielded) } else { yielded };
        if yielded.is_unmodelled() {
            return None;
        }
        if !yields.yielded.contains(&yielded) {
            yields.yielded.push(yielded);
        }
        if delegate {
            let sent = crate::checks::function::generator_next_type_argument(&operand_type)
                .unwrap_or(Type::GenuineUnknown);
            if !yields.sent.contains(&sent) {
                yields.sent.push(sent);
            }
        }
    }
    let body_flow = crate::flow::analyze_function_body_flow(body);
    crate::checks::function::inferred_generator_type(Some(&yields), None, &body_flow, contextual_return, is_async, true, ctx)
}

/// A function expression — an object-literal method among them — whose
/// written return type is a type predicate keeps its written signature on
/// the handle, as a declared member does (`DeclaredMemberSignature`), so a
/// guard that calls it through a property (`Utils.isA(node)`) can narrow.
/// So does a generic one whose signature is written out whole, so a call
/// through a property (`o.wrap("a")`) instantiates its type parameters.
pub(crate) fn with_written_predicate(
    function_type: surge_ts_types::FunctionType,
    arrow_function: &ParsedArrowFunction,
    ctx: &CheckerContext,
) -> surge_ts_types::FunctionType {
    let written_generic = !arrow_function.type_parameters.is_empty()
        && arrow_function.return_type.is_some()
        && arrow_function.parameters.iter().all(|parameter| parameter.declared_type.is_some());
    if !(written_generic
        || matches!(arrow_function.return_type, Some(surge_ts_syntax::ParsedType::Predicate(_))))
        || function_type.declaration().is_some()
    {
        return function_type;
    }
    function_type.with_declaration(std::sync::Arc::new(crate::checks::call::DeclaredMemberSignature {
        signature: crate::checks::function::function_signature_info(
            &arrow_function.type_parameters,
            &arrow_function.parameters,
            arrow_function.return_type.as_ref(),
            &ctx.file_name,
        ),
        outer_type_arguments: Vec::new(),
        generic_shape: None,
    }))
}

/// tsc widens the *fresh* literal a body expression returns, so `() => ''`
/// infers `() => string` and an object literal's property declared that way is
/// assignable from any `() => string`. A literal that came from a `const` is not
/// fresh and keeps its own type, so freshness is decided syntactically here, the
/// same way the generic-argument path decides it.
fn widen_fresh_literal_return(expression: &ParsedExpression, ty: Type) -> Type {
    if matches!(
        expression,
        ParsedExpression::StringLiteral(_)
            | ParsedExpression::NumberLiteral(_)
            | ParsedExpression::BooleanLiteral(_)
            | ParsedExpression::ObjectLiteral { .. }
            | ParsedExpression::ArrayLiteral { .. }
    ) {
        crate::checks::expr::widen_type(&ty)
    } else {
        ty
    }
}

/// tsc checks a callback's returned call against the contextual return type
/// the generic call it is passed to gives it (`instantiateContextualType`
/// through the return mapper), so `cache(() => raw())` under a
/// `Promise<string[][]>` context infers `raw`'s own type argument from that
/// rather than from its default.
fn call_typed_by_contextual_return(
    expression: &ParsedExpression,
    contextual: &Type,
    is_async: bool,
    locals: &SymbolTable,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    if !matches!(expression, ParsedExpression::Call { .. } | ParsedExpression::PropertyCall { .. })
        || contextual.is_unmodelled()
    {
        return None;
    }
    let checkpoint = ctx.diagnostics().len();
    let evaluate = |ctx: &mut CheckerContext| {
        crate::checks::expected::evaluate_expression_with_expected_type(
            expression,
            None,
            Some(contextual),
            crate::checks::expected::ExpectedTypeDiagnostic::ContextOnly,
            locals,
            ctx,
        )
    };
    let typed = if is_async {
        crate::checks::call::with_awaited_context(contextual, || evaluate(ctx))
    } else {
        evaluate(ctx)
    };
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    match typed {
        InferredExpression::Known(ty)
            if !ty.is_unmodelled() && !crate::checks::expr::carries_leaked_type_parameter(&ty, ctx) =>
        {
            Some(ty)
        }
        _ => None,
    }
}

/// [`widen_fresh_literal_return`] under a contextual return type: tsc's
/// `checkExpressionForMutableLocation` keeps a literal the contextual type is
/// literal-like for (`getWidenedLiteralLikeTypeForContextualType`), and an
/// object literal's properties each take their contextual type from the
/// contextual type's property of that name. Everything else widens as it does
/// without one.
fn widen_literals_for_contextual_return(expression: &ParsedExpression, ty: Type, contextual: &Type) -> Type {
    if matches!(
        expression,
        ParsedExpression::StringLiteral(_) | ParsedExpression::NumberLiteral(_) | ParsedExpression::BooleanLiteral(_)
    ) {
        return crate::checks::function::widen_unit_return_type(ty, Some(contextual));
    }
    let widened = widen_fresh_literal_return(expression, ty.clone());
    let kept = match (expression, &ty, &widened) {
        (ParsedExpression::ObjectLiteral { properties, .. }, Type::Object(written), Type::Object(object)) => {
            object_literal_members_kept_by_context(properties, written, object, contextual)
        }
        _ => None,
    };
    kept.unwrap_or(widened)
}

/// The widened object literal `widened` with each literal-valued property put
/// back to its `written` type where the contextual property type keeps it;
/// `None` when no property keeps anything.
fn object_literal_members_kept_by_context(
    properties: &[surge_ts_syntax::ParsedObjectProperty],
    written: &surge_ts_types::ObjectType,
    widened: &surge_ts_types::ObjectType,
    contextual: &Type,
) -> Option<Type> {
    let mut members = (*widened.properties).clone();
    let mut kept = false;
    for property in properties {
        if property.is_spread
            || property.is_method
            || property.is_accessor
            || !matches!(
                property.value,
                ParsedExpression::StringLiteral(_)
                    | ParsedExpression::NumberLiteral(_)
                    | ParsedExpression::BooleanLiteral(_)
                    | ParsedExpression::ObjectLiteral { .. }
            )
        {
            continue;
        }
        let (Some(written_member), Some(contextual_member)) = (
            written.properties.get(property.name.as_str()),
            contextual_property_type(contextual, &property.name),
        ) else {
            continue;
        };
        if contextual_member.is_unknown() {
            continue;
        }
        let member_type =
            widen_literals_for_contextual_return(&property.value, written_member.ty.clone(), &contextual_member);
        if let Some(member) = members.get_mut(property.name.as_str())
            && member.ty != member_type
        {
            member.ty = member_type;
            kept = true;
        }
    }
    if !kept {
        return None;
    }
    let mut rebuilt = crate::metrics::alloc_object_type(members, widened.string_index_type.as_deref().cloned());
    if widened.synthetic_open_index {
        rebuilt = rebuilt.with_open_index_marker();
    }
    if widened.non_primitive {
        rebuilt = rebuilt.with_non_primitive_marker();
    }
    if let Some(call_signature) = widened.call_signature() {
        rebuilt = rebuilt.with_call_signature(call_signature.clone());
    }
    if let Some(construct_signature) = widened.construct_signature() {
        rebuilt = rebuilt.with_construct_signature(construct_signature.clone());
    }
    Some(Type::Object(rebuilt))
}

/// tsc's `getTypeOfPropertyOfContextualType`: a union contextual type gives a
/// property what those of its members that have it give it.
fn contextual_property_type(contextual: &Type, name: &str) -> Option<Type> {
    match contextual.peeled() {
        Type::Union(union) => {
            let members: Vec<Type> = union
                .types()
                .iter()
                .filter_map(|member| member.get_property_access_type(name))
                .collect();
            (!members.is_empty()).then(|| surge_ts_types::union_type(members))
        }
        _ => contextual.get_property_access_type(name),
    }
}

/// A contextual parameter type is usable only once it is a real type: an
/// unresolved shape or a still-open type parameter would type the body as
/// something the caller cannot bind, which is worse than the `any` the
/// un-contextualized sketch uses.
fn contextual_parameter_type(contextual_parameters: &[Type], index: usize) -> Type {
    match contextual_parameters.get(index) {
        Some(ty)
            if !ty.is_unknown()
                && !matches!(ty, Type::TypeParameter(_))
                && !matches!(ty.peeled(), Type::Unknown) =>
        {
            ty.clone()
        }
        _ => Type::Any,
    }
}

fn primitive_declared_return_type(ty: &surge_ts_syntax::ParsedType) -> Option<Type> {
    match ty {
        surge_ts_syntax::ParsedType::String => Some(Type::String),
        surge_ts_syntax::ParsedType::Number => Some(Type::Number),
        surge_ts_syntax::ParsedType::Boolean => Some(Type::Boolean),
        surge_ts_syntax::ParsedType::Any => Some(Type::Any),
        surge_ts_syntax::ParsedType::Unknown => Some(Type::Unknown),
        surge_ts_syntax::ParsedType::UnknownKeyword => Some(Type::GenuineUnknown),
        surge_ts_syntax::ParsedType::Undefined => Some(Type::Undefined),
        surge_ts_syntax::ParsedType::Void => Some(Type::Void),
        _ => None,
    }
}

fn body_locals(
    arrow_function: &ParsedArrowFunction,
    parameter_types: &[Type],
    symbols: &SymbolTable,
) -> SymbolTable {
    // A destructured parameter binds its names from the parameter's type the
    // way the checking pass does; skipping it left `({ ctx }) => …` with `ctx`
    // unbound, so the body's return type was unknowable.
    let mut scopes = crate::symbols::ScopeStack::from_root(
        symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
    );
    // A named `function` expression's own name shadows the enclosing scope's;
    // the sketch being inferred is its type, so it stands at the sentinel.
    if let Some(name) = &arrow_function.name {
        scopes.insert_current(
            name.as_str(),
            crate::symbols::SymbolInfo {
                ty: Type::Unknown,
                kind: crate::symbols::SymbolKind::Function,
                function_signature: None,
            },
        );
    }
    for (parameter, ty) in arrow_function.parameters.iter().zip(parameter_types) {
        crate::checks::function::insert_binding_name(
            &parameter.binding_name,
            ty.clone(),
            &mut scopes,
        );
    }
    scopes
        .visible_symbols()
        .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext)
}

pub(crate) fn required_parameter_count(
    parameters: &[surge_ts_syntax::ParsedFunctionParameter],
) -> usize {
    let mut required = parameters.len();

    while required > 0 {
        let parameter = &parameters[required - 1];
        if parameter.optional || parameter.initializer.is_some() {
            required -= 1;
        } else {
            break;
        }
    }

    required
}

/// Infers an unannotated block-bodied arrow's return type from its `return`
/// statements. Deliberately narrow: a run of local declarations followed by
/// returns is the shape a callback argument almost always has, and it is what
/// lets a lazy initializer (`useState(() => { const rows = …; return rows; })`)
/// contribute its type to the call's inference. Anything with branching, a bare
/// `return;`, or a binding pattern yields `None`, keeping the previous
/// `Unknown` — the body would need real flow analysis to type honestly.
/// Whether a function body contains a `return` anywhere, nested statements
/// included. A nested `function` declaration is its own body and does not count.
fn body_contains_return(body: &[surge_ts_syntax::ParsedFunctionBodyStatement]) -> bool {
    use surge_ts_syntax::ParsedFunctionBodyStatement as Statement;

    body.iter().any(|statement| match statement {
        Statement::Return(_) => true,
        Statement::Block(statements) => body_contains_return(statements),
        Statement::If(statement) => {
            body_contains_return(&statement.then_body) || body_contains_return(&statement.else_body)
        }
        Statement::While(statement) => body_contains_return(&statement.body),
        Statement::ForOf(statement) => body_contains_return(&statement.body),
        Statement::Switch(statement) => statement
            .cases
            .iter()
            .any(|case| body_contains_return(&case.consequent)),
        Statement::Try(statement) => {
            body_contains_return(&statement.block)
                || statement
                    .handler
                    .as_ref()
                    .is_some_and(|handler| body_contains_return(&handler.body))
                || body_contains_return(&statement.finalizer)
        }
        _ => false,
    })
}

fn infer_block_body_return_type(
    body: &[surge_ts_syntax::ParsedFunctionBodyStatement],
    mut locals: SymbolTable,
    return_context: Option<(&Type, bool)>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    use surge_ts_syntax::ParsedFunctionBodyStatement;

    let returns_here = body
        .iter()
        .any(|statement| matches!(statement, ParsedFunctionBodyStatement::Return(_)));
    if !returns_here {
        return None;
    }
    // Any statement that can carry control flow (or a return this walk would
    // miss) disqualifies the body.
    let straight_line = body.iter().all(|statement| {
        matches!(
            statement,
            ParsedFunctionBodyStatement::VariableDeclaration(_)
                | ParsedFunctionBodyStatement::Return(_)
                | ParsedFunctionBodyStatement::Expression(_)
                | ParsedFunctionBodyStatement::TypeAlias(_)
        )
    });
    if !straight_line {
        return None;
    }

    let mut returned = Vec::new();
    for statement in body {
        match statement {
            ParsedFunctionBodyStatement::VariableDeclaration(variable) => {
                if variable.declared_type.is_some() {
                    return None;
                }
                let initializer = variable.initializer.as_ref()?;
                let InferredExpression::Known(ty) = infer_expression(initializer, &locals, ctx)
                else {
                    return None;
                };
                let kind = match variable.kind {
                    surge_ts_syntax::ParsedVariableKind::Var => crate::symbols::SymbolKind::Var,
                    surge_ts_syntax::ParsedVariableKind::Let => crate::symbols::SymbolKind::Let,
                    surge_ts_syntax::ParsedVariableKind::Const => {
                        crate::symbols::SymbolKind::Const
                    }
                };
                // The binding holds what its declaration widens the initializer
                // to (`const r = { ok: true }` is `{ ok: boolean }`), as the
                // checking pass binds it.
                let ty = crate::checks::var::widen_implicit_variable_initializer_type(
                    kind,
                    initializer,
                    &ty,
                    false,
                );
                let _ = locals.insert(
                    variable.name.clone(),
                    crate::symbols::SymbolInfo {
                        ty,
                        kind,
                        function_signature: None,
                    },
                );
            }
            ParsedFunctionBodyStatement::Return(statement) => {
                let expression = statement.expression.as_ref()?;
                if let Some((contextual, is_async)) = return_context
                    && let Some(ty) = call_typed_by_contextual_return(expression, contextual, is_async, &locals, ctx)
                {
                    returned.push(ty);
                    continue;
                }
                let ty = infer_expression(expression, &locals, ctx).flowing_type()?;
                if ty.is_unmodelled() && !matches!(ty, Type::ErrorType) {
                    return None;
                }
                returned.push(widen_fresh_literal_return(expression, ty));
            }
            _ => {}
        }
    }

    match returned.len() {
        0 => None,
        1 => returned.pop(),
        _ if crate::program::in_check_phase()
            && !returned.iter().any(|member| matches!(member, Type::Object(_))) =>
        {
            Some(surge_ts_types::subtype_reduced_union(
                returned
                    .into_iter()
                    .map(|member| (member, surge_ts_types::LiteralShape::Regular))
                    .collect(),
            ))
        }
        _ => Some(surge_ts_types::union_type(returned)),
    }
}
