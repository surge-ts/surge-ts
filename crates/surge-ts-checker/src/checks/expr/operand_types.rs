//! Operand rules that reject a type outright, ported from
//! `tsc/internal/checker/checker.go`: the object-literal spread
//! (`isValidSpreadType`) and the left-hand side of `instanceof`
//! (`checkInstanceOfExpression`).

use surge_ts_diagnostics::Diagnostic;
use surge_ts_syntax::TextSpan as SyntaxTextSpan;
use surge_ts_types::Type;

use crate::context::{CheckerContext, convert_span};
use crate::infer::InferredExpression;

/// tsc's `removeDefinitelyFalsyTypes`, which `isValidSpreadType` applies before
/// testing: `{ ...maybeObject }` is legal because the `undefined` half is
/// dropped, while `{ ...undefined }` is not, because dropping it leaves nothing.
fn without_definitely_falsy(ty: &Type) -> Option<Type> {
    if is_definitely_falsy(ty) {
        return None;
    }
    let Type::Union(union) = ty else {
        return Some(ty.clone());
    };
    let kept: Vec<Type> = union
        .types()
        .iter()
        .filter(|member| !is_definitely_falsy(member))
        .cloned()
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(surge_ts_types::union_type(kept))
}

fn is_definitely_falsy(ty: &Type) -> bool {
    match ty {
        Type::Undefined | Type::Null | Type::Void | Type::Never => true,
        Type::BooleanLiteral(false) => true,
        Type::StringLiteral(value) => value.is_empty(),
        Type::NumberLiteral(value) => value.value == "0",
        _ => false,
    }
}

/// tsc's `isValidSpreadType`: an object, `object`, `any`, or an instantiable
/// non-primitive; a union only when every surviving constituent qualifies.
fn is_valid_spread_type(ty: &Type) -> bool {
    let ty = base_constraint_or_type(ty);
    let Some(ty) = without_definitely_falsy(&ty) else {
        return false;
    };
    match &ty {
        Type::Any
        | Type::Object(_)
        | Type::Array(_)
        | Type::Tuple(_)
        | Type::OpenTuple(_)
        | Type::Function(_)
        | Type::TypeParameter(_) => true,
        // A modelling failure is not evidence of a bad spread, and tsc's error
        // type is an `any`.
        Type::Unknown | Type::ErrorType => true,
        Type::Reference(reference) => is_valid_spread_type(&reference.resolve()),
        Type::Union(union) => union.types().iter().all(is_valid_spread_type),
        _ => false,
    }
}

/// `getBaseConstraintOrType`, as `isValidSpreadType` maps a spread through
/// it: a type variable of the body being checked stands for its constraint,
/// and an intersection holding one for the intersection of its operands'
/// constraints, where a variable with none contributes nothing
/// (`computeBaseConstraint`). `T & undefined` is `undefined` there.
fn base_constraint_or_type(ty: &Type) -> Type {
    let constraint_of = |member: &Type| -> Option<Option<Type>> {
        let Type::TypeParameter(parameter) = member else {
            return None;
        };
        surge_ts_types::type_variable::active_constraint(parameter)
    };
    match ty {
        Type::TypeParameter(_) => match constraint_of(ty) {
            Some(Some(constraint)) if !constraint.is_type_variable() => constraint,
            _ => ty.clone(),
        },
        Type::Object(object)
            if object
                .intersection_operands
                .as_deref()
                .is_some_and(|operands| operands.iter().any(|operand| constraint_of(operand).is_some())) =>
        {
            let operands = object.intersection_operands.as_deref().unwrap_or_default();
            let mut falsy = None;
            for operand in operands {
                let operand = match constraint_of(operand) {
                    Some(Some(constraint)) => constraint,
                    Some(None) => continue,
                    None => operand.clone(),
                };
                if is_definitely_falsy(&operand) {
                    falsy = Some(operand);
                }
            }
            falsy.unwrap_or_else(|| ty.clone())
        }
        Type::Union(union) => surge_ts_types::union_type(union.types().iter().map(base_constraint_or_type).collect()),
        _ => ty.clone(),
    }
}

/// `{ ...source }` where `source` is not an object type — TS2698. tsc reports it
/// on the spread element, not on the object literal or the argument.
pub(crate) fn check_object_spread_type(
    spread_result: &InferredExpression,
    spread_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(ty), Some(span)) = (spread_result, spread_span) else {
        return;
    };
    // A type parameter constrained to `any` or `unknown` has `unknown` for its
    // base constraint (`getConstraintFromTypeParameter`), which no spread takes.
    let top_constrained = matches!(ty, Type::TypeParameter(parameter)
        if ctx.type_parameter_constraint(&parameter.name).is_some_and(|constraint| {
            matches!(constraint, surge_ts_syntax::ParsedType::Any | surge_ts_syntax::ParsedType::UnknownKeyword)
        }));
    if is_valid_spread_type(ty) && !top_constrained {
        return;
    }
    let file_name = ctx.file_name.clone();
    ctx.push(Diagnostic::ts2698(file_name).with_span(convert_span(span)));
}

/// tsc's rule is `!isTypeAny(left) && allTypesAssignableToKind(left, Primitive)`.
/// `unknown` is deliberately exempt — tsc notes the related error was already
/// reported — and so is a union with any non-primitive constituent.
fn is_all_primitive(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_)
        | Type::Undefined
        | Type::Null
        | Type::Void => true,
        Type::Reference(reference) => is_all_primitive(&reference.resolve()),
        Type::Union(union) => union.types().iter().all(is_all_primitive),
        _ => false,
    }
}

/// `x instanceof C` where `x` is a primitive — TS2358, reported on the left
/// operand.
pub(crate) fn check_instanceof_left_operand(
    left_result: &InferredExpression,
    left_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(ty), Some(span)) = (left_result, left_span) else {
        return;
    };
    if ty.is_unknown() || !is_all_primitive(ty) {
        return;
    }
    let file_name = ctx.file_name.clone();
    ctx.push(Diagnostic::ts2358(file_name).with_span(convert_span(span)));
}

/// tsc's `resolveInstanceofExpression` failure — TS2359, on the right
/// operand: not `any`, no `[Symbol.hasInstance]`, no call or construct
/// signature, and not a `Function`. Only the shapes surge sees whole are
/// judged: primitives, and object types that carry none of those members.
pub(crate) fn check_instanceof_right_operand(
    right_result: &InferredExpression,
    right_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(ty), Some(span)) = (right_result, right_span) else {
        return;
    };
    if !cannot_be_instanceof_target(ty) {
        return;
    }
    let file_name = ctx.file_name.clone();
    ctx.push(Diagnostic::ts2359(file_name).with_span(convert_span(span)));
}

/// tsc's `resolveInstanceofExpression` over an object type's own
/// `[Symbol.hasInstance]` method: `x instanceof o` is the call
/// `o[Symbol.hasInstance](x)`, rejected at the left operand (TS2860), and
/// what the method returns has to be a boolean (TS2861).
pub(crate) fn check_instanceof_has_instance(
    left_result: &InferredExpression,
    right_result: &InferredExpression,
    left_span: Option<SyntaxTextSpan>,
    right_span: Option<SyntaxTextSpan>,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(left), InferredExpression::Known(right)) = (left_result, right_result) else {
        return;
    };
    let Some(method) = has_instance_method(right) else {
        return;
    };
    if method.overloads().is_some() || !crate::checks::call::own_type_parameter_names(&method).is_empty() {
        return;
    }
    let parameter = match method.parameters() {
        [] => None,
        [rest] if method.is_variadic() => match rest.peeled() {
            Type::Array(element) => Some(*element),
            _ => None,
        },
        [first, ..] => Some(first.clone()),
    };
    if let (Some(parameter), Some(span)) = (parameter, left_span)
        && !crate::checks::call::type_contains_unknown(left)
        && !crate::checks::call::type_contains_unknown(&parameter)
        && !crate::checks::call::as_source(|| crate::checks::function::type_contains_degradation(left))
        && !surge_ts_types::is_assignable_to(left, &parameter)
    {
        ctx.push(Diagnostic::ts2860(ctx.file_name.clone()).with_span(convert_span(span)));
    }
    let returned = method.return_type();
    if let Some(span) = right_span
        && !crate::checks::call::type_contains_unknown(returned)
        && !surge_ts_types::is_assignable_to(returned, &Type::Boolean)
    {
        ctx.push(Diagnostic::ts2861(ctx.file_name.clone()).with_span(convert_span(span)));
    }
}

fn has_instance_method(ty: &Type) -> Option<surge_ts_types::FunctionType> {
    match ty {
        Type::Reference(reference) => has_instance_method(&reference.resolve()),
        Type::Object(object) => match &object.get_property("[Symbol.hasInstance]")?.ty {
            Type::Function(method) => Some(method.clone()),
            Type::Object(method) => method.call_signature().cloned(),
            _ => None,
        },
        _ => None,
    }
}

fn cannot_be_instanceof_target(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => cannot_be_instanceof_target(&reference.resolve()),
        Type::Object(object) => {
            object.construct_signature.is_none()
                && object.call_signature.is_none()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && !object.is_intersection
                && !object
                    .alias_name
                    .as_deref()
                    .is_some_and(|name| name.ends_with("Function"))
                && !object.properties.keys().any(|name| {
                    matches!(name.as_ref(), "apply" | "call" | "bind") || name.contains("hasInstance")
                })
        }
        // Without strictNullChecks `null` and `undefined` are subtypes of
        // every type, `Function` included (`isTypeSubtypeOf`).
        Type::Null | Type::Undefined if !surge_ts_types::strict_null_checks() => false,
        other => !other.is_unknown() && is_all_primitive(other),
    }
}

/// tsc's `getIteratedTypeOrElementType` failure (TS2488) for a type surge can
/// see has no iteration protocol. `nullish_is_error` separates a spread, where
/// `undefined` and `unknown` are themselves not iterable, from `for…of`, where
/// tsc reports them as possibly-nullish or unknown operands instead.
pub(crate) fn check_iterable_operand(
    operand_result: &InferredExpression,
    operand_span: Option<SyntaxTextSpan>,
    nullish_is_error: bool,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(ty), Some(span)) = (operand_result, operand_span) else {
        return;
    };
    if !is_definitely_not_iterable(ty, nullish_is_error) {
        if let Some((code, method)) = super::iteration::iterator_method_diagnostic(ty) {
            let diagnostic = match code {
                2767 => Diagnostic::ts2767(method, ctx.file_name.clone()),
                _ => Diagnostic::ts2490(method, ctx.file_name.clone()),
            };
            ctx.push(diagnostic.with_span(convert_span(span)));
        }
        return;
    }
    let file_name = ctx.file_name.clone();
    // Without a global `Iterable` tsc falls back to `isArrayLikeType` and
    // `getIterationDiagnosticDetails`: a `for…of` operand with no string
    // constituent is TS2495. A string constituent (TS2461) or an ES2015
    // iterable name (TS2802) keeps the protocol message.
    let diagnostic = if !nullish_is_error
        && ctx.ambient_global_type_declarations.get("Iterable").is_none()
        && !has_string_like_constituent(ty)
        && !is_es2015_or_later_iterable_name(&ty.name())
    {
        Diagnostic::ts2495(ty.name(), file_name)
    } else {
        Diagnostic::ts2488(ty.name(), file_name)
    };
    ctx.push(diagnostic.with_span(convert_span(span)));
}

fn has_string_like_constituent(ty: &Type) -> bool {
    match ty {
        Type::Union(union) => union.types().iter().any(has_string_like_constituent),
        Type::String | Type::StringLiteral(_) => true,
        _ => false,
    }
}

fn is_es2015_or_later_iterable_name(name: &str) -> bool {
    matches!(
        name,
        "Float32Array"
            | "Float64Array"
            | "Int16Array"
            | "Int32Array"
            | "Int8Array"
            | "NodeList"
            | "Uint16Array"
            | "Uint32Array"
            | "Uint8Array"
            | "Uint8ClampedArray"
    )
}

pub(crate) fn is_definitely_not_iterable(ty: &Type, nullish_is_error: bool) -> bool {
    match ty {
        Type::Number
        | Type::NumberLiteral(_)
        | Type::Boolean
        | Type::BooleanLiteral(_)
        | Type::BigInt
        | Type::Symbol
        | Type::Never => true,
        // A signature type carries no `[Symbol.iterator]` (`yield* fn`).
        Type::Function(_) => true,
        Type::Undefined | Type::Null | Type::GenuineUnknown => nullish_is_error,
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| is_definitely_not_iterable(member, nullish_is_error)),
        // An intersection has every operand's members: one iterable operand
        // (`T & any[]` under `Array.isArray`) makes it iterable.
        Type::Object(_) if surge_ts_types::type_variable::is_narrowed_type_variable(ty) => {
            let Type::Object(object) = ty else { return false };
            object
                .intersection_operands
                .as_deref()
                .is_some_and(|operands| {
                    operands.iter().all(|operand| is_definitely_not_iterable(operand, nullish_is_error))
                })
        }
        // `getIterationTypesOfIterable` looks the protocol member up as a
        // property, which no index signature answers; only an index surge
        // opened over an operand it could not enumerate proves nothing.
        // An optional protocol member reads as possibly `undefined` under
        // `strictNullChecks`, which is not a method to call.
        Type::Object(object) => {
            (object
                .get_property(surge_ts_types::ITERATION_PROTOCOL_MEMBER)
                .is_none_or(|member| member.optional && surge_ts_types::strict_null_checks())
                || iterator_method_returns_no_iterator(object))
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
                && !(object.synthetic_open_index && object.string_index_type.is_some())
                // An interface extending `Array` inherits its iterator, which
                // surge's collapsed `Array` heritage does not carry over.
                && !(object.number_index_type.is_some() && object.get_property("length").is_some())
        }
        // The lib's async iteration interfaces declare `[Symbol.asyncIterator]`
        // and inherit no `[Symbol.iterator]`.
        Type::Reference(_) if is_lib_async_iterable_reference(ty) => true,
        // A declaration file's interface (the lib's `ArrayIterator`, say) can
        // inherit its protocol member through heritage surge does not resolve
        // in full; only a source-declared type is judged.
        Type::Reference(reference)
            if reference
                .id
                .split('\u{0}')
                .next()
                .is_some_and(|file| !file.is_empty() && !crate::modules::is_declaration_file_name(file)) =>
        {
            match ty.peeled() {
                peeled @ Type::Object(_) => is_definitely_not_iterable(&peeled, nullish_is_error),
                _ => false,
            }
        }
        _ => false,
    }
}

/// `getIterationTypesOfIterator`: what `[Symbol.iterator]()` returns must
/// have a `next` method. Only a fully modelled object return without any
/// `next` is judged.
fn iterator_method_returns_no_iterator(object: &surge_ts_types::ObjectType) -> bool {
    let Some(member) = object.get_property(surge_ts_types::ITERATION_PROTOCOL_MEMBER) else {
        return false;
    };
    let Type::Function(method) = member.ty.peeled() else {
        return false;
    };
    if method.overloads().is_some() || crate::checks::function::type_contains_degradation(method.return_type()) {
        return false;
    }
    match method.return_type().peeled() {
        Type::Object(iterator) => {
            iterator.get_property("next").is_none()
                && iterator.string_index_type.is_none()
                && iterator.call_signature().is_none()
                && !iterator.is_intersection
        }
        _ => false,
    }
}

const ASYNC_ITERATION_PROTOCOL_MEMBER: &str = "[Symbol.asyncIterator]";

fn is_lib_async_iterable_reference(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    reference.id.split('\u{0}').next().is_some_and(crate::modules::is_declaration_file_name)
        && matches!(
            reference.id.rsplit('\u{0}').next(),
            Some("AsyncIterable" | "AsyncIterableIterator" | "AsyncGenerator" | "AsyncIteratorObject")
        )
}

/// tsc's `getIteratedTypeOrElementType` failure for an operand that may be
/// async or sync iterable: a `for await…of` operand, or a `yield*` operand in
/// an async generator. With the global `Iterable` declared it is TS2504;
/// without it tsc falls back to `isArrayLikeType`, which only `for await`
/// (which also takes strings) reports here, as TS2495. `for await` runs
/// `checkNonNullExpression` first, so a nullish operand is not reported here;
/// `yield*` does not.
pub(crate) fn check_async_iterable_operand(
    operand_result: &InferredExpression,
    operand_span: Option<SyntaxTextSpan>,
    for_await: bool,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(ty), Some(span)) = (operand_result, operand_span) else {
        return;
    };
    if !is_definitely_not_async_iterable(ty, !for_await) {
        return;
    }
    let file_name = ctx.file_name.clone();
    let diagnostic = if ctx.ambient_global_type_declarations.get("Iterable").is_some() {
        Diagnostic::ts2504(ty.name(), file_name)
    } else if for_await
        && !has_string_like_constituent(ty)
        && !is_es2015_or_later_iterable_name(&ty.name())
    {
        Diagnostic::ts2495(ty.name(), file_name)
    } else {
        return;
    };
    ctx.push(diagnostic.with_span(convert_span(span)));
}

/// [`is_definitely_not_iterable`] with async iterables allowed
/// (`getIterationTypesOfIterableWorker`): neither an `[Symbol.asyncIterator]()`
/// nor an `[Symbol.iterator]()` method is callable without arguments.
fn is_definitely_not_async_iterable(ty: &Type, nullish_is_error: bool) -> bool {
    match ty {
        Type::Union(union) => union
            .types()
            .iter()
            .any(|member| is_definitely_not_async_iterable(member, nullish_is_error)),
        Type::Object(object) if !surge_ts_types::type_variable::is_narrowed_type_variable(ty) => {
            !has_protocol_method(object, ASYNC_ITERATION_PROTOCOL_MEMBER)
                && !has_protocol_method(object, surge_ts_types::ITERATION_PROTOCOL_MEMBER)
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
                && !(object.synthetic_open_index && object.string_index_type.is_some())
                && !(object.number_index_type.is_some() && object.get_property("length").is_some())
        }
        Type::Reference(reference)
            if reference
                .id
                .split('\u{0}')
                .next()
                .is_some_and(|file| !file.is_empty() && !crate::modules::is_declaration_file_name(file)) =>
        {
            match ty.peeled() {
                peeled @ Type::Object(_) => is_definitely_not_async_iterable(&peeled, nullish_is_error),
                _ => false,
            }
        }
        _ if is_lib_async_iterable_reference(ty) => false,
        _ => is_definitely_not_iterable(ty, nullish_is_error),
    }
}

/// The iteration that sends a value to an iterator's `next`, which the
/// message names.
#[derive(Clone, Copy)]
pub(crate) enum IterationSend {
    ForOf,
    Spread,
    ArrayDestructuring,
    YieldStar,
}

/// The `checkAssignability` half of tsc's `getIteratedTypeOrElementType`:
/// what the iteration sends each step — `undefined`, or for `yield*` the
/// containing generator's next type — must be assignable to the next type of
/// the operand's iterator (TS2763-TS2766). Only a reference to a lib
/// iterable or generator interface says what its `next` takes (the
/// iteration-types fast path); any other operand is left alone.
pub(crate) fn check_iteration_next_type(
    operand_result: &InferredExpression,
    operand_span: Option<SyntaxTextSpan>,
    sent: &Type,
    send: IterationSend,
    allow_async: bool,
    ctx: &mut CheckerContext,
) {
    let (InferredExpression::Known(ty), Some(span)) = (operand_result, operand_span) else {
        return;
    };
    let Some(next) = lib_iterable_next_type(ty, allow_async) else {
        return;
    };
    // The deep degradation walk forces lazy references, so it only runs once
    // the relation has failed.
    if matches!(next, Type::Unknown | Type::ErrorType | Type::TypeParameter(_))
        || surge_ts_types::is_assignable_to(sent, &next)
        || crate::checks::function::type_contains_degradation(&next)
        || crate::checks::function::type_contains_degradation(sent)
    {
        return;
    }
    let (sent_name, next_name, file_name) = (sent.name(), next.name(), ctx.file_name.clone());
    let diagnostic = match send {
        IterationSend::ForOf => Diagnostic::ts2763(sent_name, next_name, file_name),
        IterationSend::Spread => Diagnostic::ts2764(sent_name, next_name, file_name),
        IterationSend::ArrayDestructuring => Diagnostic::ts2765(sent_name, next_name, file_name),
        IterationSend::YieldStar => Diagnostic::ts2766(sent_name, next_name, file_name),
    };
    ctx.push(diagnostic.with_span(convert_span(span)));
}

/// The `TNext` of a reference to a lib iterable or generator interface,
/// defaulted as its declaration defaults it. The async ones only count where
/// async iterables are allowed.
fn lib_iterable_next_type(ty: &Type, allow_async: bool) -> Option<Type> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    let mut parts = reference.id.split('\u{0}');
    let file = parts.next()?;
    let name = parts.next_back()?;
    if !crate::modules::is_declaration_file_name(file) {
        return None;
    }
    let default = match name {
        "Iterable" | "IterableIterator" | "Generator" => Type::Any,
        "IteratorObject" => Type::GenuineUnknown,
        "AsyncIterable" | "AsyncIterableIterator" | "AsyncGenerator" if allow_async => Type::Any,
        "AsyncIteratorObject" if allow_async => Type::GenuineUnknown,
        _ => return None,
    };
    Some(reference.arguments.get(2).cloned().unwrap_or(default))
}

/// Whether `object` has the protocol method `name` with a signature callable
/// without arguments; `getIterationTypesOfIterableSlow` ignores the others
/// (`[Symbol.asyncIterator](n: number)` makes nothing iterable).
fn has_protocol_method(object: &surge_ts_types::ObjectType, name: &str) -> bool {
    match object.get_property_type(name) {
        None => false,
        Some(Type::Function(function)) => {
            function.overloads().is_some() || function.required_parameter_count() == 0
        }
        Some(_) => true,
    }
}
