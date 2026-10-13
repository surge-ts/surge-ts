use super::*;

use surge_ts_syntax::{
    ParsedFunctionType, ParsedFunctionTypeParameter, ParsedObjectType, ParsedTupleElement,
};
use surge_ts_types::{ObjectProperty, PropertyMap};

use crate::metrics::{alloc_function_type, alloc_object_type};

pub(crate) fn resolve_tuple_type(
    elements: Vec<ParsedType>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let mut resolved_elements = Vec::new();
    let mut had_error = false;
    let min_length = ParsedType::tuple_min_length(&elements);
    // Under `exactOptionalPropertyTypes` an optional element's type is the
    // written one (tsc adds `missing`, not `undefined`); the recorded
    // `minLength` keeps it optional.
    let exact_optional = surge_ts_types::strict_null_checks() && surge_ts_types::exact_optional_property_types();

    for element in elements {
        let element = match element {
            ParsedType::Union(members) if exact_optional && members.len() == 2 && matches!(members[0], ParsedType::Undefined) => {
                members[1].clone()
            }
            other => other,
        };
        let resolved_element = resolve_parsed_type(element, ctx, resolving, substitution);
        had_error |= resolved_element.had_error;
        resolved_elements.push(resolved_element.ty);
    }

    ResolvedType {
        ty: surge_ts_types::written_tuple_type(resolved_elements, min_length),
        had_error,
    }
}

/// A tuple written with a spread element (`[...a, ...b]`, `[head, ...tail]`).
/// When every spread operand resolves to a known-length tuple the whole tuple
/// has a known length, so the operands splice in place and the result is an
/// ordinary fixed tuple — which is what makes `[...path, 0]` and the
/// `[head, ...tail]` list idiom evaluate instead of degrading. A length-less
/// operand (an array, an unresolved parameter) leaves the length unknown; the
/// fixed-length model cannot state that, so it degrades as it did before this
/// shape was modelled at all.
/// `SURGE_READONLY_ARRAYS=0` collapses `ReadonlyArray<T>` to the mutable array
/// again; the parser reads the same switch for the `readonly` operator.
pub(crate) fn readonly_arrays_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_READONLY_ARRAYS").as_deref() != Ok("0"))
}

struct ReadonlyShape(Type);

impl surge_ts_types::ResolveReference for ReadonlyShape {
    fn resolve(&self) -> Type {
        self.0.clone()
    }
}

/// Wraps an array or tuple shape in the nominal `readonly` reference. Anything
/// else (a degraded operand, a union the operator was written over in error)
/// is returned unchanged so no new degradation is introduced.
pub(crate) fn readonly_reference(ty: Type) -> Type {
    if let Some(readonly) = surge_ts_types::type_variable::readonly_generic_tuple(&ty) {
        return readonly;
    }
    match ty {
        Type::Reference(ref reference) if reference.is_readonly_array() => ty,
        _ if matches!(ty, Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_)) || surge_ts_types::is_written_tuple(&ty) => {
            let display = format!("readonly {}", ty.name());
            Type::Reference(surge_ts_types::TypeReference::new(
                surge_ts_types::READONLY_REFERENCE_ID,
                display,
                vec![ty.clone()],
                std::sync::Arc::new(ReadonlyShape(ty)),
            ))
        }
        other => other,
    }
}

pub(crate) fn resolve_variadic_tuple_type(
    elements: Vec<ParsedTupleElement>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let mut resolved = Vec::with_capacity(elements.len());
    let mut had_error = false;
    let first_rest_span = elements.iter().find_map(|element| match element {
        ParsedTupleElement::Rest(_, span) => *span,
        ParsedTupleElement::Fixed(_) => None,
    });

    let mut rest_reported = false;
    for element in elements {
        let (is_rest, written, span) = match element {
            ParsedTupleElement::Fixed(written) => (false, written, None),
            ParsedTupleElement::Rest(written, span) => (true, written, span),
        };
        let resolved_element = resolve_parsed_type(written, ctx, resolving, substitution);
        had_error |= resolved_element.had_error;
        // tsc's `checkTupleType`: a rest element's type must be array-like,
        // reported for the first offender only (TS2574).
        if is_rest
            && !rest_reported
            && let Some(span) = span
            && !ctx.suppress_unknown_type_name()
            && is_array_like_type(&resolved_element.ty) == Some(false)
        {
            rest_reported = true;
            ctx.push_utility_diagnostic_once(
                surge_ts_diagnostics::Diagnostic::ts2574(ctx.file_name.clone())
                    .with_span(crate::context::convert_span(span)),
            );
        }
        resolved.push((is_rest, resolved_element.ty));
    }

    // `checkCrossProductUnion` in `createNormalizedTupleType`: spreading
    // unions of tuples makes one tuple per combination. tsc anchors on the
    // tuple node, which surge does not keep; the first spread stands for it.
    let cross_product = resolved.iter().fold(1usize, |product, (is_rest, ty)| {
        product.saturating_mul(match ty {
            Type::Union(union) if *is_rest => union.types().len().max(1),
            _ => 1,
        })
    });
    if cross_product >= 100_000 {
        if let Some(span) = first_rest_span {
            ctx.push(
                surge_ts_diagnostics::Diagnostic::ts2590(ctx.file_name.clone())
                    .with_span(crate::context::convert_span(span)),
            );
        }
        return ResolvedType {
            ty: Type::ErrorType,
            had_error,
        };
    }

    if let Some(generic) = generic_tuple_from_operands(&resolved) {
        return ResolvedType { ty: generic, had_error };
    }
    if let Some(members) = splice_spread_operands(&resolved) {
        return ResolvedType {
            ty: Type::Tuple(members),
            had_error,
        };
    }
    ResolvedType {
        ty: open_tuple_from_operands(&resolved).unwrap_or(Type::Unknown),
        had_error,
    }
}

/// A tuple spreading a type variable of the body being checked stays generic
/// (`createNormalizedTupleType` keeps a variadic element over a generic
/// type); the other spreads contribute their elements.
fn generic_tuple_from_operands(elements: &[(bool, Type)]) -> Option<Type> {
    use surge_ts_types::type_variable::TupleElementKind;
    if !elements.iter().any(|(is_rest, ty)| *is_rest && ty.is_type_variable()) {
        return None;
    }
    let mut tuple = Vec::with_capacity(elements.len());
    for (is_rest, ty) in elements {
        if !*is_rest {
            tuple.push((TupleElementKind::Required, ty.clone()));
        } else if ty.is_type_variable() {
            tuple.push((TupleElementKind::Variadic, ty.clone()));
        } else {
            tuple.extend(surge_ts_types::type_variable::spread_elements(ty)?);
        }
    }
    surge_ts_types::type_variable::generic_tuple_variable(tuple, false)
}

/// tsc's `isArrayLikeType`: an array, or a non-nullable type assignable to
/// `readonly any[]`. `None` for a type surge could not resolve.
fn is_array_like_type(ty: &Type) -> Option<bool> {
    if let Type::Reference(reference) = ty
        && reference.is_readonly_array()
    {
        return Some(true);
    }
    match ty.peeled() {
        Type::Unknown | Type::ErrorType => None,
        Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) | Type::Any | Type::Never => Some(true),
        Type::GenuineUnknown | Type::Null | Type::Undefined => Some(false),
        Type::TypeParameter(parameter) => match surge_ts_types::type_variable::active_constraint(&parameter) {
            Some(Some(constraint)) => is_array_like_type(&constraint),
            Some(None) => Some(false),
            None => None,
        },
        Type::Union(union) => union
            .types()
            .iter()
            .try_fold(true, |all, member| Some(all && is_array_like_type(member)?)),
        other => Some(surge_ts_types::is_assignable_to(
            &other,
            &Type::Array(Box::new(Type::Any)),
        )),
    }
}

/// `[a, ...b[], c]` with exactly one length-less spread — an array, or an open
/// tuple whose own fixed slots join the outer ones — is an open tuple. A spread
/// of an unresolved operand, or a second length-less one, still has no shape.
fn open_tuple_from_operands(elements: &[(bool, Type)]) -> Option<Type> {
    let mut leading = Vec::new();
    let mut rest: Option<Type> = None;
    let mut trailing = Vec::new();
    for (is_rest, ty) in elements {
        let slots = if rest.is_none() { &mut leading } else { &mut trailing };
        if !*is_rest {
            slots.push(ty.clone());
            continue;
        }
        if let Some(members) = spread_operand_members(ty) {
            slots.extend(members);
            continue;
        }
        if rest.is_some() {
            return None;
        }
        let peeled = match ty {
            Type::Reference(_) => crate::program::with_dts_expansion_reason(
                crate::program::DtsExpansionReason::ConditionalType,
                || ty.peeled(),
            ),
            other => other.clone(),
        };
        match peeled {
            Type::Array(element) => rest = Some(*element),
            Type::OpenTuple(inner) => {
                leading.extend(inner.leading);
                rest = Some(*inner.rest);
                trailing.extend(inner.trailing);
            }
            // `createNormalizedTupleType` spreads `any` as an `any` rest
            // (`[any, ...any]` is `[any, ...any[]]`).
            Type::Any => rest = Some(Type::Any),
            _ => return None,
        }
    }
    let rest = rest?;
    Some(Type::OpenTuple(surge_ts_types::OpenTupleType {
        leading,
        rest: Box::new(rest),
        trailing,
    }))
}

fn splice_spread_operands(elements: &[(bool, Type)]) -> Option<Vec<Type>> {
    let mut members = Vec::with_capacity(elements.len());

    for (is_rest, ty) in elements {
        if !*is_rest {
            members.push(ty.clone());
            continue;
        }
        members.extend(spread_operand_members(ty)?);
    }

    Some(members)
}

/// The elements a `...T` operand contributes, or `None` when its length is not
/// known. A deferred alias instantiation carries its tuple behind a lazy
/// reference, so the operand is peeled before giving up.
fn spread_operand_members(ty: &Type) -> Option<Vec<Type>> {
    match ty {
        Type::Tuple(members) => Some(members.clone()),
        Type::Reference(_) => match crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::ConditionalType,
            || ty.peeled(),
        ) {
            Type::Tuple(members) => Some(members),
            _ => None,
        },
        _ => None,
    }
}

/// checker.go `getTypePredicateOfSignature` for a written predicate return,
/// its type produced by `resolve_type`. Its type's own diagnostics belong to
/// the return annotation's resolution.
pub(crate) fn written_type_predicate(
    return_type: &ParsedType,
    value_parameter_names: impl Iterator<Item = Option<String>>,
    ctx: &mut CheckerContext,
    resolve_type: impl FnOnce(&ParsedType, &mut CheckerContext) -> Type,
) -> Option<surge_ts_types::TypePredicate> {
    use surge_ts_types::TypePredicateKind;
    let ParsedType::Predicate(predicate) = return_type else {
        return None;
    };
    let this_predicate = predicate.parameter_name == "this";
    let kind = match (this_predicate, predicate.asserts) {
        (true, false) => TypePredicateKind::This,
        (true, true) => TypePredicateKind::AssertsThis,
        (false, false) => TypePredicateKind::Identifier,
        (false, true) => TypePredicateKind::AssertsIdentifier,
    };
    let parameter_index = if this_predicate {
        None
    } else {
        let mut names = value_parameter_names;
        Some(names.position(|name| name.as_deref() == Some(predicate.parameter_name.as_str()))?)
    };
    let diagnostics_before = ctx.diagnostics().len();
    let ty = predicate.ty.as_ref().map(|ty| resolve_type(ty, ctx));
    ctx.truncate_diagnostics(diagnostics_before);
    Some(surge_ts_types::TypePredicate { kind, parameter_index, ty })
}

/// A written predicate's type. Inside another declaration's resolution it is
/// deferred like a parameter annotation: nothing else resolves it there, and
/// it often names a class that extends the one being resolved, which would
/// close a cycle that degrades both.
fn predicate_type(
    ty: &ParsedType,
    return_type: &ParsedType,
    top_level: bool,
    substitution: &TypeParameterSubstitution,
    ctx: &mut CheckerContext,
) -> Type {
    if top_level {
        return resolve_parsed_type(ty.clone(), ctx, &mut Vec::new(), substitution).ty;
    }
    let written_at = match return_type {
        ParsedType::Predicate(predicate) => predicate.type_span.map_or(0, |span| span.start),
        _ => 0,
    };
    // The key is by position, so the written type and the bindings it is
    // resolved under keep two predicates at one offset apart.
    let written = {
        use std::hash::Hasher;
        let mut hasher = surge_ts_types::fx::FxHasher::default();
        hasher.write(format!("{ty:?}").as_bytes());
        hasher.finish()
    };
    let name = format!(
        "type-predicate {written:016x} {:016x}",
        super::super::cache::member_substitution_fingerprint(substitution)
    );
    crate::infer::make_lazy_signature_annotation_reference(
        ctx,
        &name,
        written_at,
        crate::infer::LazySignatureComponent::TypePredicate,
        ty.clone(),
        crate::infer::LazySignatureEnvironment::for_member_substitution(substitution),
    )
}

/// relater.go `isInstantiatedGenericParameter` for a signature resolved under
/// an enclosing declaration's bindings: the value parameters written as one of
/// its type variables (`set(value: T)`), or an indexed access into one.
fn instantiated_generic_parameters(
    function_type: &ParsedFunctionType,
    substitution: &TypeParameterSubstitution,
) -> u32 {
    let is_bound_variable = |ty: &ParsedType| {
        let mut ty = ty;
        while let ParsedType::IndexedAccess(access) = ty {
            ty = &access.object_type;
        }
        matches!(ty, ParsedType::Named(named)
            if named.type_arguments.is_empty()
                && !function_type.type_parameters.iter().any(|own| own.name == named.name)
                && substitution.get(&named.name).is_some()
                && !substitution.is_placeholder(&named.name))
    };
    function_type
        .parameters
        .iter()
        .filter(|parameter| !parameter.is_this)
        .take(32)
        .enumerate()
        .filter(|(_, parameter)| is_bound_variable(&parameter.ty))
        .fold(0, |mask, (index, _)| mask | (1 << index))
}

/// A signature's written `this` parameter, resolved; `None` when it has none
/// or surge could not model it.
fn written_this_type(
    function_type: &ParsedFunctionType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> Option<(Type, bool)> {
    let written = function_type.parameters.iter().find(|parameter| parameter.is_this)?;
    // The parameter's own annotation reports where the signature is checked.
    let checkpoint = ctx.diagnostics().len();
    let resolved = resolve_parsed_type(written.ty.clone(), ctx, resolving, substitution);
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    (!resolved.had_error && !resolved.ty.is_unmodelled()).then_some((resolved.ty, written.polymorphic_this))
}

/// A method's written `this` return, which the parser lowers to the
/// polymorphic `this` name, or a body that returns only `this`, whose inferred
/// return answers the declaring class until a member read binds the receiver.
pub(crate) fn returns_polymorphic_this(return_type: &ParsedType) -> bool {
    match return_type {
        ParsedType::Named(named) => {
            named.name == surge_ts_syntax::POLYMORPHIC_THIS_TYPE_NAME && named.type_arguments.is_empty()
        }
        ParsedType::InferredMember(member) => matches!(&member.source,
            surge_ts_syntax::ParsedInferredMemberSource::MethodBody(method) if method.returns_only_this),
        _ => false,
    }
}

pub(crate) fn resolve_function_type(
    function_type: std::sync::Arc<ParsedFunctionType>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let local_substitution = extend_substitution_with_type_parameters(
        substitution,
        &function_type.type_parameters,
        &function_type.parameters,
        ctx,
        resolving,
    );

    // A written constraint or default (`<T extends C = D>() => …`) is a type
    // reference like any other, resolved for its own diagnostics — but only
    // where it is written. Resolving it again beneath another declaration's
    // expansion buys no diagnostic and walks the constraint's own generic
    // graph at every use site, which on zod's `.d.ts` chains does not
    // terminate in reasonable time.
    let written_here = resolving.is_empty();
    for type_parameter in function_type.type_parameters.iter().take_while(|_| written_here) {
        for written in [&type_parameter.constraint, &type_parameter.default_type]
            .into_iter()
            .flatten()
        {
            let _ = super::resolve_parsed_type(written.clone(), ctx, resolving, &local_substitution);
        }
    }

    let value_parameters = function_type
        .parameters
        .iter()
        .filter(|parameter| !parameter.is_this)
        .cloned()
        .collect::<Vec<_>>();
    let required_parameter_count = required_parameter_count(&value_parameters);
    let is_variadic = value_parameters
        .last()
        .is_some_and(|parameter| parameter.rest);
    let (parameters, return_type, had_error) =
        resolve_signature_types(&function_type, ctx, resolving, &local_substitution);
    let this_type = written_this_type(&function_type, ctx, resolving, &local_substitution);
    let top_level = resolving.is_empty();
    let mut resolved_function = alloc_function_type(
        parameters,
        return_type,
        is_variadic,
        required_parameter_count,
    )
    .with_parameter_names(written_parameter_names(&value_parameters))
    .with_type_parameter_head(crate::checks::function::type_parameter_head(
        &function_type.type_parameters,
    ))
    .with_instantiated_generic_parameters(instantiated_generic_parameters(&function_type, substitution))
    .with_type_predicate(written_type_predicate(
        &function_type.return_type,
        value_parameters.iter().map(|parameter| parameter.name.clone()),
        ctx,
        |ty, ctx| predicate_type(ty, &function_type.return_type, top_level, &local_substitution, ctx),
    ))
    .with_this_type(this_type.as_ref().map(|(ty, _)| ty.clone()))
    .with_polymorphic_this(this_type.is_some_and(|(_, polymorphic)| polymorphic))
    .with_polymorphic_this_return(returns_polymorphic_this(&function_type.return_type));
    // A generic signature's own type parameters are erased above (`T` maps to
    // the sentinel), so a call through the resolved handle could not infer
    // them — every `find<T>(type: Type<T>): Collection<T>` on an interface
    // returned `unknown`. The written signature rides on the handle, with the
    // enclosing bindings the body was resolved under, so the call site
    // re-instantiates it from its arguments.
    let mentions_this = !returns_polymorphic_this(&function_type.return_type)
        && crate::checks::call::parsed_type_mentions_name(
            &function_type.return_type,
            surge_ts_syntax::POLYMORPHIC_THIS_TYPE_NAME,
        );
    resolved_function = resolved_function.with_return_mentioning_polymorphic_this(mentions_this);
    if (!function_type.type_parameters.is_empty()
        || mentions_this
        || matches!(*function_type.return_type, ParsedType::Predicate(_)))
        && let Some(mut declared) =
            crate::checks::call::DeclaredMemberSignature::capture(&function_type, substitution, ctx)
    {
        if declared.outer_type_arguments.is_empty() && !ctx.is_library_scoped_file(&ctx.file_name) {
            declared.generic_shape = generic_signature_shape(&function_type, substitution, ctx, resolving);
        }
        resolved_function = resolved_function.with_declaration(std::sync::Arc::new(declared));
    }
    if function_type.type_parameters.is_empty() {
        resolved_function = crate::checks::call::declared_without_type_parameters(resolved_function);
    }
    ResolvedType {
        ty: Type::Function(resolved_function),
        had_error,
    }
}

/// A signature's parameter and return types under `substitution`, and whether
/// any failed to resolve. A parameter's name is in scope for the parameters
/// after it and for the return type: `(x: number) => typeof x`,
/// `({ a: alias }: T) => typeof alias`.
fn resolve_signature_types(
    function_type: &ParsedFunctionType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> (Vec<Type>, Type, bool) {
    let mut parameters = Vec::new();
    let mut had_error = false;
    let outer_parameter_bindings = ctx.signature_parameter_bindings.clone();
    // Every parameter is in scope in every annotation of the list; one not
    // resolved yet (itself, a later one) reads as the sentinel until it is.
    for parameter in function_type.parameters.iter().filter(|parameter| !parameter.is_this) {
        if parameter.bound_names.is_empty() {
            if let Some(name) = &parameter.name {
                ctx.signature_parameter_bindings.push((name.clone(), Type::Unknown));
            }
        } else {
            for bound in &parameter.bound_names {
                ctx.signature_parameter_bindings.push((bound.name.clone(), Type::Unknown));
            }
        }
    }
    for parameter in function_type.parameters.iter().cloned() {
        let is_this = parameter.is_this;
        let name = parameter.name.clone();
        let bound_names = parameter.bound_names.clone();
        let is_rest = parameter.rest;
        let resolved_parameter =
            resolve_function_type_parameter(parameter, ctx, resolving, substitution);
        had_error |= resolved_parameter.had_error;
        if !is_this {
            let bound_type = if is_rest {
                Type::Array(Box::new(resolved_parameter.ty.clone()))
            } else {
                resolved_parameter.ty.clone()
            };
            if bound_names.is_empty() {
                if let Some(name) = name {
                    ctx.signature_parameter_bindings.push((name, bound_type));
                }
            } else {
                for bound in &bound_names {
                    let ty = crate::checks::function::bound_name_type(&bound_type, bound);
                    ctx.signature_parameter_bindings.push((bound.name.clone(), ty));
                }
            }
        }
        // The `this` parameter is resolved so an unresolved `this` type still
        // reports once (and propagates `had_error` to avoid a cascade), but it is
        // not a real call parameter, so it is excluded from arity and arguments.
        if is_this {
            continue;
        }
        parameters.push(resolved_parameter.ty);
    }

    let return_type =
        resolve_parsed_type((*function_type.return_type).clone(), ctx, resolving, substitution);
    ctx.signature_parameter_bindings = outer_parameter_bindings;
    (parameters, return_type.ty, had_error || return_type.had_error)
}

/// tsc relates a generic signature through its own type parameters
/// (`compareSignaturesRelated`), which the resolved signature erases to the
/// degradation sentinel once constrained (see
/// `extend_substitution_with_type_parameters`). The written shape keeps them as
/// placeholders, with each constraint resolved over them. It is taken only for
/// a signature written in the program's own files where no enclosing
/// instantiation binds anything, so each written signature is resolved for it
/// once, and not at all when a type parameter has a default or anything in it
/// fails to resolve.
fn generic_signature_shape(
    function_type: &ParsedFunctionType,
    substitution: &TypeParameterSubstitution,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
) -> Option<std::sync::Arc<surge_ts_types::GenericSignatureShape>> {
    let type_parameters = &function_type.type_parameters;
    if !type_parameters.iter().any(|parameter| parameter.constraint.is_some())
        || type_parameters.iter().any(|parameter| parameter.default_type.is_some())
    {
        return None;
    }
    let mut shape_substitution =
        substitution.clone_with_reason(surge_ts_types::TypeCopyReason::SubstitutionChanged);
    for parameter in type_parameters {
        shape_substitution.insert(parameter.name.clone(), Type::type_parameter(&parameter.name));
    }
    let diagnostics_before = ctx.diagnostics().len();
    let mut had_error = false;
    let mut constrained = Vec::with_capacity(type_parameters.len());
    for parameter in type_parameters {
        let constraint = match parameter.constraint.clone() {
            Some(constraint) => {
                let resolved = resolve_parsed_type(constraint, ctx, resolving, &shape_substitution);
                had_error |= resolved.had_error;
                Some(resolved.ty)
            }
            None => None,
        };
        constrained.push((std::sync::Arc::<str>::from(parameter.name.as_str()), constraint));
    }
    let (parameters, return_type, signature_had_error) =
        resolve_signature_types(function_type, ctx, resolving, &shape_substitution);
    // The written signature already reported its diagnostics.
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    if had_error || signature_had_error {
        return None;
    }
    surge_ts_types::install_generic_signature_shape_lookup(
        crate::checks::call::declared_generic_signature_shape,
    );
    Some(std::sync::Arc::new(surge_ts_types::GenericSignatureShape {
        type_parameters: constrained,
        parameters,
        return_type,
    }))
}

/// The names as written, so a diagnostic can render `(value: string) => void`
/// the way tsc does. Display-only: they are attached to the type handle and
/// never reach the interned payload or assignability.
pub(crate) fn written_parameter_names(
    parameters: &[surge_ts_syntax::ParsedFunctionTypeParameter],
) -> Vec<Option<std::sync::Arc<str>>> {
    parameters
        .iter()
        .map(|parameter| parameter.name.as_deref().map(std::sync::Arc::<str>::from))
        .collect()
}

/// [`resolve_function_type`] with lazy components (Stage 2 of member-level
/// lazy expansion): the FunctionType shell — arity, variadic, required count,
/// `this`/rest handling — is built exactly as the eager path builds it, but an
/// eligible parameter or return annotation becomes a lazy component reference
/// resolved on first read. `this` and rest parameters always resolve eagerly
/// (`this` is typing metadata outside arity; a rest annotation is consumed by
/// variadic call checking, which a deferred wrapper would mis-shape).
pub(crate) fn resolve_function_type_lazy_components(
    function_type: std::sync::Arc<ParsedFunctionType>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
    interface_name: &str,
    declaration_start: usize,
    member_index: usize,
    member_name: &str,
) -> ResolvedType {
    let local_substitution = extend_substitution_with_type_parameters(
        substitution,
        &function_type.type_parameters,
        &function_type.parameters,
        ctx,
        resolving,
    );
    let mut substitution_fingerprint: Option<u64> = None;

    let value_parameters = function_type
        .parameters
        .iter()
        .filter(|parameter| !parameter.is_this)
        .cloned()
        .collect::<Vec<_>>();
    let required_parameter_count = required_parameter_count(&value_parameters);
    let is_variadic = value_parameters
        .last()
        .is_some_and(|parameter| parameter.rest);
    let mut parameters = Vec::new();
    let mut had_error = false;

    let mut value_index = 0usize;
    for parameter in function_type.parameters.iter() {
        let is_this = parameter.is_this;
        let is_rest = parameter.rest;
        let defer = !is_this && !is_rest && defer_method_component_annotation(&parameter.ty);
        if defer {
            let ty = super::super::cache::make_lazy_method_component_reference(
                ctx,
                super::super::cache::LazyMemberIdentity {
                    interface_name,
                    declaration_start,
                    member_index,
                    member_name,
                    component: Some(crate::infer::LazySignatureComponent::Parameter(value_index)),
                    substitution_fingerprint: *substitution_fingerprint.get_or_insert_with(|| {
                        super::super::cache::member_substitution_fingerprint(&local_substitution)
                    }),
                },
                &parameter.ty,
                &local_substitution,
            );
            parameters.push(ty);
            value_index += 1;
            continue;
        }
        let resolved_parameter =
            resolve_function_type_parameter(parameter.clone(), ctx, resolving, &local_substitution);
        had_error |= resolved_parameter.had_error;
        if is_this {
            continue;
        }
        value_index += 1;
        parameters.push(resolved_parameter.ty);
    }

    // The return annotation stays eager: a call's result flows through the
    // whole program — truthiness narrowing, unions, optional chains — and a
    // deferred `X | undefined` return measurably escapes narrowing (25 tRPC
    // false positives). Parameters have a narrow consumer surface (argument
    // assignability and contextual typing, both of which peel).
    let return_type = resolve_parsed_type(
        (*function_type.return_type).clone(),
        ctx,
        resolving,
        &local_substitution,
    );
    had_error |= return_type.had_error;
    let this_type = written_this_type(&function_type, ctx, resolving, &local_substitution);
    let mut resolved_function =
        alloc_function_type(parameters, return_type.ty, is_variadic, required_parameter_count)
            .with_instantiated_generic_parameters(instantiated_generic_parameters(&function_type, substitution))
            .with_type_predicate(written_type_predicate(
                &function_type.return_type,
                value_parameters.iter().map(|parameter| parameter.name.clone()),
                ctx,
                |ty, ctx| predicate_type(ty, &function_type.return_type, false, &local_substitution, ctx),
            ))
            .with_this_type(this_type.as_ref().map(|(ty, _)| ty.clone()))
    .with_polymorphic_this(this_type.is_some_and(|(_, polymorphic)| polymorphic))
    .with_polymorphic_this_return(returns_polymorphic_this(&function_type.return_type));
    if function_type.type_parameters.is_empty() {
        resolved_function = crate::checks::call::declared_without_type_parameters(resolved_function);
    }
    ResolvedType {
        ty: Type::Function(resolved_function),
        had_error,
    }
}

/// The method-component deferral tier: structured shapes minus anything
/// containing `typeof` (resolved against value tables the captured
/// environment drops) and minus predicates (`x is T` shapes the signature).
fn defer_method_component_annotation(annotation: &ParsedType) -> bool {
    match annotation {
        ParsedType::Object(_)
        | ParsedType::Tuple(_)
        | ParsedType::Union(_)
        | ParsedType::Intersection(_)
        | ParsedType::Function(_)
        | ParsedType::KeyOf(_)
        | ParsedType::IndexedAccess(_)
        | ParsedType::Mapped(_)
        | ParsedType::Conditional(_)
        | ParsedType::TemplateLiteral(_) => !crate::modules::annotation_contains_typeof(annotation),
        ParsedType::Array(element) => defer_method_component_annotation(element),
        _ => false,
    }
}

pub(crate) fn required_parameter_count(
    parameters: &[surge_ts_syntax::ParsedFunctionTypeParameter],
) -> usize {
    let mut required = parameters.len();

    while required > 0 {
        let parameter = &parameters[required - 1];
        if parameter.optional || parameter.rest {
            required -= 1;
        } else {
            break;
        }
    }

    required
}

pub(crate) fn resolve_function_type_parameter(
    parameter: ParsedFunctionTypeParameter,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let ParsedFunctionTypeParameter { ty, .. } = parameter;
    let resolved = resolve_parsed_type(ty, ctx, resolving, substitution);
    ResolvedType {
        ty: resolved.ty,
        had_error: resolved.had_error,
    }
}

/// Borrows the parsed object rather than consuming it: every `ParsedType`
/// payload is `Arc`-backed, so cloning a member annotation is a refcount bump
/// while unwrapping the shared object literal would deep-copy its whole
/// property list on each of the hundreds of thousands of resolutions.
/// A fold of same-named type-literal members is whichever overload it equals,
/// handle and all, and carries no member list: one marked as taking no type
/// arguments must not answer for a group whose members differ in that.
fn without_mixed_arity_marker(
    merged: surge_ts_types::FunctionType,
    left: &surge_ts_types::FunctionType,
    right: &surge_ts_types::FunctionType,
) -> surge_ts_types::FunctionType {
    let takes_none = |function: &surge_ts_types::FunctionType| {
        function.declaration().is_some_and(|declaration| {
            declaration.is::<crate::checks::call::DeclaredWithoutTypeParameters>()
        })
    };
    if !takes_none(&merged) || (takes_none(left) && takes_none(right)) {
        return merged;
    }
    alloc_function_type(
        merged.parameters().to_vec(),
        merged.return_type().clone(),
        merged.is_variadic(),
        merged.required_parameter_count(),
    )
}

pub(crate) fn resolve_object_type(
    object_type: &ParsedObjectType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let mut properties = PropertyMap::default();
    let mut had_error = false;
    // A member of a type literal is a structural crossing like an interface
    // member: a declaration that re-enters itself through one is legal
    // recursion (see `CheckerContext::type_literal_member_frames`).
    let literal_frame = resolving.len();
    ctx.structural_resolution_frames.push(literal_frame);
    ctx.type_literal_member_frames.push(literal_frame);

    let mut method_overload_members: Vec<(&str, Vec<surge_ts_types::FunctionType>)> = Vec::new();
    for property in &object_type.properties {
        let property_type = resolve_parsed_type(property.ty.clone(), ctx, resolving, substitution);
        had_error |= property_type.had_error;

        // Same-named function members of one type literal are overloads, exactly
        // as they are in an interface body, so they fold into one permissive
        // signature the same way. Without this the last declaration won and every
        // earlier overload was dropped, which reported a call matching an earlier
        // overload's arity against the last one's (ts-pattern's `.with`).
        if let Some(existing) = properties.get(property.name.as_str())
            && let Type::Function(existing_fn) = &existing.ty
            && let Type::Function(incoming) = &property_type.ty
        {
            match method_overload_members
                .iter_mut()
                .find(|(name, _)| *name == property.name.as_str())
            {
                Some((_, members)) => members.push(incoming.clone()),
                None => method_overload_members
                    .push((property.name.as_str(), vec![existing_fn.clone(), incoming.clone()])),
            }
            let merged = without_mixed_arity_marker(
                crate::infer::types::interface::merge_overload_signatures(existing_fn, incoming),
                existing_fn,
                incoming,
            );
            let optional = existing.optional && property.optional;
            let method = existing.method || property.is_method;
            properties.insert(
                property.name.as_str().into(),
                if optional {
                    ObjectProperty::optional(Type::Function(merged))
                } else {
                    ObjectProperty::required(Type::Function(merged))
                }
                .with_method(method),
            );
            continue;
        }

        let object_property = if property.optional {
            ObjectProperty::optional(property_type.ty)
        } else {
            ObjectProperty::required(property_type.ty)
        }
        .with_method(property.is_method)
        .with_readonly(property.readonly);

        properties.insert(property.name.as_str().into(), object_property);
    }
    // The members ride along with the fold, as an interface's do, so a call
    // resolves against the overload whose arity fits.
    for (name, overloads) in method_overload_members {
        if let Some(property) = properties.get_mut(name)
            && let Type::Function(merged) = &property.ty
        {
            property.ty = Type::Function(merged.clone().with_overloads(overloads));
        }
    }

    // An index signature whose value type is in error stays (tsc's
    // `errorType` value), the way a property's does: a self-query cut short
    // (`{ [n: number]: typeof m }`) is the degraded type, not `{}`.
    let string_index_type = object_type
        .string_index_type
        .as_deref()
        .map(|index_type| {
            let resolved = resolve_parsed_type(index_type.clone(), ctx, resolving, substitution);
            had_error |= resolved.had_error;
            if resolved.had_error && !resolved.ty.is_unknown() { Type::Unknown } else { resolved.ty }
        });

    let number_index_type = object_type
        .number_index_type
        .as_deref()
        .map(|index_type| {
            let resolved = resolve_parsed_type(index_type.clone(), ctx, resolving, substitution);
            had_error |= resolved.had_error;
            if resolved.had_error && !resolved.ty.is_unknown() { Type::Unknown } else { resolved.ty }
        });

    had_error |= crate::infer::types::late_bind_member_names(&mut properties, ctx);
    let keyed_indexes = resolve_keyed_indexes(
        object_type.keyed_indexes.as_deref(),
        string_index_type.as_ref(),
        ctx,
        resolving,
        substitution,
    );
    let mut resolved_object = alloc_object_type(properties, string_index_type)
        .with_number_index_type(number_index_type)
        .with_readonly_indexes(object_type.string_index_readonly, object_type.number_index_readonly);
    resolved_object.keyed_indexes = keyed_indexes;
    if object_type.non_primitive {
        resolved_object = resolved_object.with_non_primitive_marker();
    }
    if let Some(display_name) = &object_type.display_name {
        resolved_object = resolved_object.with_alias_name(display_name.clone());
    }
    if let Some(call_signature) = object_type.call_signature.as_deref() {
        let resolved = resolve_parsed_type(
            ParsedType::Function(std::sync::Arc::new(call_signature.clone())),
            ctx,
            resolving,
            substitution,
        );
        had_error |= resolved.had_error;
        if let Type::Function(function_type) = resolved.ty {
            // A type literal's call signatures are overloads as an interface's
            // are: the fold answers consumers that want one signature, and the
            // group is what a call resolves against.
            let mut overloads = Vec::with_capacity(object_type.call_signature_overloads.len());
            for overload in &object_type.call_signature_overloads {
                let resolved = resolve_parsed_type(
                    ParsedType::Function(std::sync::Arc::new(overload.clone())),
                    ctx,
                    resolving,
                    substitution,
                );
                had_error |= resolved.had_error;
                if let Type::Function(overload) = resolved.ty {
                    overloads.push(overload);
                }
            }
            resolved_object = resolved_object.with_call_signature(function_type.with_overloads(overloads));
        }
    }
    if let Some(construct_signature) = object_type.construct_signature.as_deref() {
        let resolved = resolve_parsed_type(
            ParsedType::Function(std::sync::Arc::new(construct_signature.clone())),
            ctx,
            resolving,
            substitution,
        );
        had_error |= resolved.had_error;
        if let Type::Function(function_type) = resolved.ty {
            // A group relates member by member (`signaturesRelatedTo`), as a
            // call signature group and an interface's construct group do.
            let mut overloads = Vec::with_capacity(object_type.construct_signature_overloads.len());
            for overload in &object_type.construct_signature_overloads {
                let resolved = resolve_parsed_type(
                    ParsedType::Function(std::sync::Arc::new(overload.clone())),
                    ctx,
                    resolving,
                    substitution,
                );
                had_error |= resolved.had_error;
                if let Type::Function(overload) = resolved.ty {
                    overloads.push(overload);
                }
            }
            let modifiers = surge_ts_types::ConstructModifiers::new(
                object_type.abstract_construct_signature,
                surge_ts_types::ConstructorAccessibility::Public,
            );
            resolved_object = resolved_object
                .with_construct_signature(function_type.with_overloads(overloads).with_construct_modifiers(modifiers));
        }
    }
    ctx.type_literal_member_frames.pop();
    ctx.structural_resolution_frames.pop();

    ResolvedType {
        ty: Type::Object(resolved_object),
        had_error,
    }
}

pub(crate) fn resolve_union_type(
    types: Vec<ParsedType>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let mut resolved_types = Vec::new();
    let mut had_error = false;

    for ty in types {
        let resolved = resolve_parsed_type(ty, ctx, resolving, substitution);
        had_error |= resolved.had_error;
        resolved_types.push(resolved.ty);
    }

    if resolved_types.is_empty() {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: true,
        };
    }

    ResolvedType {
        ty: union_type(resolved_types),
        had_error,
    }
}

/// Resolves an object's [`surge_ts_syntax::ParsedKeyedIndexes`]. Any key surge
/// cannot pin to a pattern or branded string (an alias of `string`, a numeric
/// or symbol alias, a generic) drops them all, leaving the string slot to
/// answer every key as before.
pub(crate) fn resolve_keyed_indexes(
    keyed: Option<&surge_ts_syntax::ParsedKeyedIndexes>,
    stand_in: Option<&Type>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> Option<std::sync::Arc<surge_ts_types::KeyedIndexes>> {
    fn branded_string(ty: &Type) -> bool {
        matches!(ty.peeled(), Type::Object(object) if object.is_intersection
            && object.intersection_operands.as_deref().is_some_and(|operands| {
                operands.iter().any(|operand| matches!(operand, Type::String) || branded_string(operand))
            }))
    }
    fn pattern_key(key: &Type) -> Option<Type> {
        let peeled = surge_ts_types::peel_to_pattern_literal(key);
        if surge_ts_types::is_template_literal_type(&peeled)
            || surge_ts_types::string_mapping_parts(&peeled).is_some()
        {
            return Some(peeled);
        }
        // An intersection of patterns keys what every one of them admits.
        let operands = match key {
            Type::Reference(reference) if reference.id.starts_with("\u{0}intersection") => {
                Some(reference.arguments.to_vec())
            }
            _ => match key.peeled() {
                Type::Object(object) if object.is_intersection => object.intersection_operands.as_deref().map(<[Type]>::to_vec),
                _ => None,
            },
        };
        if operands.as_deref().is_some_and(|operands| {
                operands.len() > 1 && operands.iter().all(|operand| {
                    let operand = surge_ts_types::peel_to_pattern_literal(operand);
                    surge_ts_types::is_template_literal_type(&operand)
                        || surge_ts_types::string_mapping_parts(&operand).is_some()
                })
            })
        {
            return Some(key.clone());
        }
        branded_string(key).then(|| key.clone())
    }
    let keyed = keyed?;
    let mut infos = Vec::new();
    if stand_in.is_some() {
        for signature in &keyed.signatures {
            let key = resolve_parsed_type(signature.key.clone(), ctx, resolving, substitution);
            let value = resolve_parsed_type(signature.value.clone(), ctx, resolving, substitution);
            let constituents = match key.ty.peeled() {
                Type::Union(union) => union.types().to_vec(),
                _ => vec![key.ty],
            };
            let keys: Option<Vec<Type>> = constituents.iter().map(pattern_key).collect();
            match keys {
                Some(keys) if !key.had_error && !value.had_error => {
                    infos.extend(keys.into_iter().map(|key| (key, value.ty.clone())));
                }
                _ => {
                    infos.clear();
                    break;
                }
            }
        }
    }
    let symbol = keyed.symbol.as_ref().and_then(|value| {
        let resolved = resolve_parsed_type(value.clone(), ctx, resolving, substitution);
        (!resolved.had_error).then_some(resolved.ty)
    });
    (!infos.is_empty() || symbol.is_some()).then(|| {
        std::sync::Arc::new(surge_ts_types::KeyedIndexes {
            infos,
            stand_in: stand_in.cloned(),
            symbol,
        })
    })
}
