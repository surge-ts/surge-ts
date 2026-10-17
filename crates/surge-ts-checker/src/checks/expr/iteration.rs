//! tsc's iteration protocol (`getIterationTypesOfIterable` and the iterator
//! and iterator-result helpers it drives): the *yield*, *return* and *next*
//! types of an iterable, found through the lib's iterable interfaces or
//! structurally through `[Symbol.iterator]()`, `next()`, `return()` and
//! `throw()`.

use surge_ts_types::{Type, union_type};

#[derive(Clone, Debug)]
pub(crate) struct IterationTypes {
    pub(crate) yield_type: Type,
    pub(crate) return_type: Type,
    pub(crate) next_type: Option<Type>,
}

impl IterationTypes {
    fn any() -> Self {
        Self {
            yield_type: Type::Any,
            return_type: Type::Any,
            next_type: Some(Type::Any),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Resolver {
    Sync,
    Async,
}

impl Resolver {
    fn iterator_member(self) -> &'static str {
        match self {
            Resolver::Sync => surge_ts_types::ITERATION_PROTOCOL_MEMBER,
            Resolver::Async => "[Symbol.asyncIterator]",
        }
    }

    fn resolve(self, ty: Type) -> Type {
        match self {
            Resolver::Sync => ty,
            Resolver::Async => crate::checks::call::awaited_type(&ty),
        }
    }
}

/// How an iteration reads its operand (tsc's `IterationUse` flags that pick
/// the resolvers).
#[derive(Clone, Copy)]
pub(crate) struct IterationUse {
    pub(crate) allow_async: bool,
    pub(crate) allow_sync: bool,
    /// `for await…of`, whose sync fallback is awaited only after a fast-path
    /// hit on the async resolver too (`getAsyncFromSyncIterationTypes`).
    pub(crate) for_of: bool,
    pub(crate) strict_builtin_iterator_return: bool,
}

impl IterationUse {
    pub(crate) fn sync(strict_builtin_iterator_return: bool) -> Self {
        Self {
            allow_async: false,
            allow_sync: true,
            for_of: false,
            strict_builtin_iterator_return,
        }
    }

    pub(crate) fn for_await(strict_builtin_iterator_return: bool) -> Self {
        Self {
            allow_async: true,
            allow_sync: true,
            for_of: true,
            strict_builtin_iterator_return,
        }
    }

    pub(crate) fn async_generator_delegate(strict_builtin_iterator_return: bool) -> Self {
        Self {
            allow_async: true,
            allow_sync: true,
            for_of: false,
            strict_builtin_iterator_return,
        }
    }
}

/// `getIterationTypesOfIterable`. `None` is tsc's `noIterationTypes`; a type
/// surge could not model is `None` too, so callers keep their fallbacks.
pub(crate) fn iteration_types_of_iterable(ty: &Type, usage: IterationUse) -> Option<IterationTypes> {
    if matches!(ty, Type::Any) {
        return Some(IterationTypes::any());
    }
    if ty.is_unmodelled() {
        return None;
    }
    if let Type::Union(union) = ty {
        let mut all = Vec::with_capacity(union.types().len());
        for member in union.types() {
            all.push(iteration_types_of_iterable(member, usage)?);
        }
        return Some(combine(all));
    }
    if usage.allow_async {
        if let Some(types) = iterable_fast(ty, Resolver::Async, usage) {
            return Some(if usage.for_of { async_from_sync(types) } else { types });
        }
        if let Some(types) = iterable_slow(ty, Resolver::Async, usage) {
            return Some(types);
        }
    }
    if usage.allow_sync {
        if let Some(types) = iterable_fast(ty, Resolver::Sync, usage) {
            return Some(if usage.allow_async { async_from_sync(types) } else { types });
        }
        if let Some(types) = iterable_slow(ty, Resolver::Sync, usage) {
            return Some(if usage.allow_async { async_from_sync(types) } else { types });
        }
    }
    None
}

fn combine(all: Vec<IterationTypes>) -> IterationTypes {
    let mut yields = Vec::with_capacity(all.len());
    let mut returns = Vec::with_capacity(all.len());
    let mut nexts = Vec::new();
    for types in all {
        yields.push(types.yield_type);
        returns.push(types.return_type);
        nexts.extend(types.next_type);
    }
    IterationTypes {
        yield_type: union_type(yields),
        return_type: union_type(returns),
        next_type: (!nexts.is_empty()).then(|| union_type(nexts)),
    }
}

fn async_from_sync(types: IterationTypes) -> IterationTypes {
    if matches!(
        (&types.yield_type, &types.return_type, &types.next_type),
        (Type::Any, Type::Any, Some(Type::Any))
    ) {
        return types;
    }
    IterationTypes {
        yield_type: crate::checks::call::awaited_type(&types.yield_type),
        return_type: crate::checks::call::awaited_type(&types.return_type),
        next_type: types.next_type,
    }
}

/// The name of a reference to an interface a declaration file declares.
fn lib_reference(ty: &Type) -> Option<(&str, &surge_ts_types::TypeReference)> {
    let Type::Reference(reference) = ty else {
        return None;
    };
    let mut parts = reference.id.split('\u{0}');
    let file = parts.next()?;
    let name = parts.next_back()?;
    surge_ts_syntax::is_declaration_file_name(file).then_some((name, reference))
}

fn builtin_return(usage: IterationUse) -> Type {
    if usage.strict_builtin_iterator_return {
        Type::Undefined
    } else {
        Type::Any
    }
}

fn resolved_types(
    resolver: Resolver,
    reference: &surge_ts_types::TypeReference,
    defaults: [Type; 3],
) -> IterationTypes {
    let [yield_default, return_default, next_default] = defaults;
    let argument = |index: usize, default: Type| reference.arguments.get(index).cloned().unwrap_or(default);
    IterationTypes {
        yield_type: resolver.resolve(argument(0, yield_default)),
        return_type: resolver.resolve(argument(1, return_default)),
        next_type: Some(argument(2, next_default)),
    }
}

/// The lib interfaces whose type arguments are the iteration types
/// (`getIterationTypesOfIterableFast` and `getIterationTypesOfIteratorFast`);
/// `head` is `Iterable` or `Iterator`.
fn reference_fast(ty: &Type, resolver: Resolver, head: &str, usage: IterationUse) -> Option<IterationTypes> {
    let (name, reference) = lib_reference(ty)?;
    let prefix = match resolver {
        Resolver::Sync => "",
        Resolver::Async => "Async",
    };
    let bare = name.strip_prefix(prefix)?;
    let any = || Type::Any;
    let unknown = || Type::GenuineUnknown;
    if bare == head || bare == "IterableIterator" {
        return Some(resolved_types(resolver, reference, [unknown(), any(), any()]));
    }
    if bare == "IteratorObject" {
        return Some(resolved_types(resolver, reference, [unknown(), unknown(), unknown()]));
    }
    if bare == "Generator" {
        return Some(resolved_types(resolver, reference, [unknown(), any(), any()]));
    }
    let builtin = match resolver {
        Resolver::Sync => matches!(name, "ArrayIterator" | "MapIterator" | "SetIterator" | "StringIterator"),
        Resolver::Async => name == "ReadableStreamAsyncIterator",
    };
    builtin.then(|| IterationTypes {
        yield_type: resolver.resolve(reference.arguments.first().cloned().unwrap_or(Type::GenuineUnknown)),
        return_type: resolver.resolve(builtin_return(usage)),
        next_type: Some(Type::GenuineUnknown),
    })
}

fn iterable_fast(ty: &Type, resolver: Resolver, usage: IterationUse) -> Option<IterationTypes> {
    if let Some(types) = reference_fast(ty, resolver, "Iterable", usage) {
        return Some(types);
    }
    // An array, a tuple and a string iterate through the lib's `ArrayIterator`
    // and `StringIterator`, which the fast path reads.
    if resolver == Resolver::Sync {
        let element = match ty {
            Type::Array(element) => Some(element.as_ref().clone()),
            Type::Tuple(elements) => Some(union_type(elements.to_vec())),
            Type::OpenTuple(tuple) => Some(tuple.element_union()),
            Type::String | Type::StringLiteral(_) => Some(Type::String),
            Type::Reference(reference) if reference.is_readonly_array() => match reference.resolve() {
                Type::Array(element) => Some(*element),
                _ => None,
            },
            _ => None,
        };
        if let Some(element) = element {
            return Some(IterationTypes {
                yield_type: element,
                return_type: builtin_return(usage),
                next_type: Some(Type::GenuineUnknown),
            });
        }
    }
    None
}

/// The call signatures of a member's type.
fn call_signatures(ty: &Type) -> Option<Vec<surge_ts_types::FunctionType>> {
    let mut signatures = Vec::new();
    match ty {
        Type::Function(function) => function.push_overload_members(&mut signatures),
        Type::Object(object) => object.call_signature()?.push_overload_members(&mut signatures),
        Type::Reference(_) => return call_signatures(&ty.peeled()),
        _ => return None,
    }
    Some(signatures)
}

/// tsc intersects the return types of several signatures, which surge has no
/// constructor for; it only answers when they agree.
fn common_return_type(signatures: &[surge_ts_types::FunctionType]) -> Option<Type> {
    let (first, rest) = signatures.split_first()?;
    let return_type = first.return_type().clone();
    rest.iter()
        .all(|signature| *signature.return_type() == return_type)
        .then_some(return_type)
}

/// `getIterationTypesOfIterableSlow`.
fn iterable_slow(ty: &Type, resolver: Resolver, usage: IterationUse) -> Option<IterationTypes> {
    let Type::Object(object) = ty.peeled() else {
        return None;
    };
    let method = object.get_property(resolver.iterator_member())?;
    if method.is_optional() {
        return None;
    }
    if matches!(method.ty, Type::Any) {
        return Some(IterationTypes::any());
    }
    let valid: Vec<_> = call_signatures(&method.ty)?
        .into_iter()
        .filter(|signature| signature.required_parameter_count() == 0)
        .collect();
    let iterator = common_return_type(&valid)?;
    iterator_types(&iterator, resolver, usage)
}

/// `getIterationTypesOfIteratorWorker`.
fn iterator_types(ty: &Type, resolver: Resolver, usage: IterationUse) -> Option<IterationTypes> {
    if matches!(ty, Type::Any) {
        return Some(IterationTypes::any());
    }
    if ty.is_unmodelled() {
        return None;
    }
    if let Some(types) = reference_fast(ty, resolver, "Iterator", usage) {
        return Some(types);
    }
    let Type::Object(object) = ty.peeled() else {
        return None;
    };
    let mut all = vec![method_types(&object, "next", resolver)?];
    for name in ["return", "throw"] {
        if object.get_property(name).is_some() {
            all.extend(method_types(&object, name, resolver));
        }
    }
    Some(combine(all))
}

/// `getIterationTypesOfMethod`, without the diagnostics it can report.
fn method_types(object: &surge_ts_types::ObjectType, name: &str, resolver: Resolver) -> Option<IterationTypes> {
    let method = object.get_property(name)?;
    if name == "next" && method.is_optional() {
        return None;
    }
    let method_type = if name == "next" {
        method.ty.clone()
    } else {
        surge_ts_types::remove_undefined(&method.ty)
    };
    if matches!(method_type, Type::Any) {
        return Some(IterationTypes::any());
    }
    if method_type.is_unmodelled() {
        return None;
    }
    let signatures = call_signatures(&method_type)?;
    let returned = common_return_type(&signatures)?;
    let mut returns = Vec::new();
    // The *next* type is the first parameter's (`getTypeAtPosition`), which
    // surge's lowering of a rest parameter written as a tuple union
    // (`...[value]: [] | [TNext]`) does not keep; it is left unknown.
    let next_type = None;
    if name == "return" {
        let parameters: Vec<Type> = signatures
            .iter()
            .filter_map(|signature| signature.parameters().first().cloned())
            .collect();
        let parameter = if parameters.is_empty() {
            Type::GenuineUnknown
        } else {
            union_type(parameters)
        };
        returns.push(resolver.resolve(parameter));
    }
    let resolved = resolver.resolve(returned);
    let yield_type = match iterator_result_types(&resolved) {
        Some((yield_type, return_type)) => {
            returns.push(return_type);
            yield_type
        }
        None => {
            returns.push(Type::Any);
            Type::Any
        }
    };
    Some(IterationTypes {
        yield_type,
        return_type: union_type(returns),
        next_type,
    })
}

/// `getIterationTypesOfIteratorResult`: the *yield* and *return* types, `None`
/// when neither is found. A side tsc finds nothing for is `never`, which the
/// unions it joins drop, except that a result with no done side returns
/// `void`.
fn iterator_result_types(ty: &Type) -> Option<(Type, Type)> {
    if matches!(ty, Type::Any) {
        return Some((Type::Any, Type::Any));
    }
    if ty.is_unmodelled() {
        return None;
    }
    if let Some((name, reference)) = lib_reference(ty) {
        let argument = || reference.arguments.first().cloned().unwrap_or(Type::GenuineUnknown);
        match name {
            "IteratorYieldResult" => return Some((argument(), Type::Never)),
            "IteratorReturnResult" => return Some((Type::Never, argument())),
            _ => {}
        }
    }
    let members: Vec<Type> = match ty.peeled() {
        Type::Union(union) => union.types().to_vec(),
        other => vec![other],
    };
    let mut yield_values = Vec::new();
    let mut return_values = Vec::new();
    let mut yield_complete = true;
    let mut return_complete = true;
    let mut any_yield = false;
    let mut any_return = false;
    for member in &members {
        let (yield_side, return_side) = match lib_reference(member) {
            Some(("IteratorYieldResult", _)) => (true, false),
            Some(("IteratorReturnResult", _)) => (false, true),
            _ => {
                let done = member.get_property_access_type("done").unwrap_or(Type::BooleanLiteral(false));
                (
                    surge_ts_types::is_assignable_to(&Type::BooleanLiteral(false), &done),
                    surge_ts_types::is_assignable_to(&Type::BooleanLiteral(true), &done),
                )
            }
        };
        let value = member.get_property_access_type("value");
        if yield_side {
            any_yield = true;
            match &value {
                Some(value) => yield_values.push(value.clone()),
                None => yield_complete = false,
            }
        }
        if return_side {
            any_return = true;
            match &value {
                Some(value) => return_values.push(value.clone()),
                None => return_complete = false,
            }
        }
    }
    let yield_type = (any_yield && yield_complete).then(|| union_type(yield_values));
    let return_type = (any_return && return_complete).then(|| union_type(return_values));
    if yield_type.is_none() && return_type.is_none() {
        return None;
    }
    Some((yield_type.unwrap_or(Type::Never), return_type.unwrap_or(Type::Void)))
}

/// `isArrayLikeType` for a destructuring source; `None` when surge cannot
/// tell.
fn is_array_like(ty: &Type) -> Option<bool> {
    match ty {
        Type::Any | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => Some(true),
        Type::Union(union) => {
            let mut all = true;
            for member in union.types() {
                all &= is_array_like(member)?;
            }
            Some(all)
        }
        Type::Reference(reference) if reference.is_readonly_array() => Some(true),
        Type::Reference(_) => match ty.peeled() {
            Type::Reference(_) => None,
            peeled => is_array_like(&peeled),
        },
        _ if ty.is_unmodelled() => None,
        _ => Some(false),
    }
}

/// `checkArrayLiteralDestructuringElementAssignment`: what element `index` of
/// an array destructuring assignment reads from a source of `source_type`.
/// `None` for an element of an array-like source, which reads by index; a
/// rest element always answers, `unknown` when surge cannot tell.
pub(crate) fn array_pattern_element_type(
    source_type: &Type,
    index: usize,
    rest: bool,
    strict_builtin_iterator_return: bool,
) -> Option<Type> {
    let array_like = is_array_like(source_type);
    let iterated = || {
        iteration_types_of_iterable(source_type, IterationUse::sync(strict_builtin_iterator_return))
            .map(|types| types.yield_type)
            .filter(|ty| !ty.is_unmodelled())
    };
    if !rest {
        return match array_like {
            Some(false) => iterated(),
            _ => None,
        };
    }
    let every_tuple = match source_type.peeled() {
        Type::Tuple(elements) => Some(vec![elements]),
        Type::Union(union) => union
            .types()
            .iter()
            .map(|member| match member.peeled() {
                Type::Tuple(elements) => Some(elements),
                _ => None,
            })
            .collect(),
        _ => None,
    };
    if let Some(tuples) = every_tuple {
        return Some(union_type(
            tuples
                .into_iter()
                .map(|elements| Type::tuple(elements.get(index..).map(<[Type]>::to_vec).unwrap_or_default()))
                .collect(),
        ));
    }
    let element = match array_like {
        Some(true) => Some(crate::checks::function::for_of_element_type(source_type)).filter(|ty| !ty.is_unmodelled()),
        Some(false) => iterated(),
        None => None,
    };
    Some(element.map_or(Type::Unknown, |element| Type::Array(Box::new(element))))
}

/// `getIterationTypesOfGeneratorFunctionReturnType` for an annotation that is
/// an iterable of its kind. Read as a bare iterator, tsc takes the iteration
/// types of methods inherited from the lib's `Iterator`/`Generator` from the
/// type arguments they were instantiated with, which surge's member types do
/// not record, so that reading is left to the caller.
pub(crate) fn generator_return_iteration_types(
    ty: &Type,
    is_async: bool,
    strict_builtin_iterator_return: bool,
) -> Option<IterationTypes> {
    if matches!(ty, Type::Any) {
        return Some(IterationTypes::any());
    }
    let usage = IterationUse {
        allow_async: is_async,
        allow_sync: !is_async,
        for_of: false,
        strict_builtin_iterator_return,
    };
    // An `Iterator`/`AsyncIterator` annotation is no iterable; its own
    // protocol answers (`getIterationTypesOfIterator`).
    iteration_types_of_iterable(ty, usage).or_else(|| {
        iterator_types(ty, if is_async { Resolver::Async } else { Resolver::Sync }, usage)
    })
}

/// The diagnostic `getIterationTypesOfMethod` reports for a modelled iterator
/// object an iterable's `[Symbol.iterator]()` returns: a `return` or `throw`
/// member that is not callable (TS2767), or a method whose result has no
/// `value` (TS2490). `None` wherever surge cannot see the whole protocol.
pub(crate) fn iterator_method_diagnostic(ty: &Type) -> Option<(u32, &'static str)> {
    let Type::Object(object) = ty.peeled() else {
        return None;
    };
    if object.is_intersection || object.synthetic_open_index {
        return None;
    }
    let method = object.get_property(surge_ts_types::ITERATION_PROTOCOL_MEMBER)?;
    let Type::Function(method) = method.ty.peeled() else {
        return None;
    };
    if method.overloads().is_some() || crate::checks::function::type_contains_degradation(method.return_type()) {
        return None;
    }
    let Type::Object(iterator) = method.return_type().peeled() else {
        return None;
    };
    if iterator.is_intersection || iterator.synthetic_open_index || iterator.string_index_type.is_some() {
        return None;
    }
    for name in ["next", "return", "throw"] {
        let Some(member) = iterator.get_property(name) else {
            continue;
        };
        if name == "next" && member.is_optional() {
            return None;
        }
        let member_type = surge_ts_types::remove_undefined(&member.ty);
        if member_type.is_unmodelled() || matches!(member_type, Type::Any) {
            return None;
        }
        let Some(signatures) = call_signatures(&member_type) else {
            return (name != "next").then_some((2767, name));
        };
        let returned = common_return_type(&signatures)?;
        if crate::checks::function::type_contains_degradation(&returned) {
            return None;
        }
        if name != "throw" && iterator_result_types(&returned).is_none() {
            return Some((2490, name));
        }
    }
    None
}