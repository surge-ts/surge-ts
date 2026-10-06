use super::*;

use surge_ts_syntax::{ParsedTypeParameter, TextSpan};

pub(crate) struct BoundTypeArguments {
    pub(crate) substitution: TypeParameterSubstitution,
    /// Whether any argument/default resolved with `had_error`. The binding
    /// still proceeds with the degraded type — one failed argument must not
    /// erase an otherwise-usable instantiation (a callable's degraded callback
    /// parameter would otherwise strip the whole call signature) — but the
    /// taint must reach the caller's `ResolvedType` so degraded expansions are
    /// never interned.
    pub(crate) had_error: bool,
}

/// tsc's `getMinTypeArgumentCount`: a reference must write every type argument
/// up to the last type parameter with no default.
pub(crate) fn min_type_argument_count(type_parameters: &[ParsedTypeParameter]) -> usize {
    type_parameters
        .iter()
        .rposition(|parameter| parameter.default_type.is_none())
        .map_or(0, |index| index + 1)
}

pub(crate) fn bind_type_arguments(
    type_parameters: &[ParsedTypeParameter],
    type_arguments: Vec<ParsedType>,
    name: &str,
    name_span: Option<TextSpan>,
    argument_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    parent_substitution: &TypeParameterSubstitution,
    pre_resolved: Option<&[Type]>,
    pre_resolved_errors: Option<&[bool]>,
    declaration_scope: Option<(
        &Option<std::sync::Arc<crate::symbols::TypeDeclarationScope>>,
        &str,
    )>,
) -> Option<BoundTypeArguments> {
    let mut bound_had_error = false;
    if type_parameters.is_empty() {
        if !type_arguments.is_empty() {
            emit_type_is_not_generic(name, name_span, ctx);
            return None;
        }

        return Some(BoundTypeArguments {
            substitution: TypeParameterSubstitution::new(),
            had_error: false,
        });
    }

    let min_type_argument_count = min_type_argument_count(type_parameters);
    if type_arguments.len() > type_parameters.len() {
        emit_generic_arity(
            name,
            min_type_argument_count,
            type_parameters.len(),
            name_span,
            ctx,
        );
        return None;
    }

    let mut substitution = TypeParameterSubstitution::new();
    // `checkTypeArgumentConstraints` relates the written reference once and
    // stops at its first failure. Re-resolved under an instantiating
    // substitution (a conditional's `infer` bindings included) it is not that
    // node, and tsc reports nothing for it.
    // Too few arguments is TS2314 and the reference is the error type, whose
    // constraints tsc never checks.
    let mut constraints_settled = type_parameters
        .iter()
        .all(|parameter| parameter.constraint.is_none())
        || type_arguments.len() < min_type_argument_count
        || type_arguments
            .iter()
            .any(|argument| names_instantiated_parameter(argument, parent_substitution));

    for (index, parameter) in type_parameters.iter().enumerate() {
        if let Some(argument) = type_arguments.get(index) {
            // Reuse the caller's already-resolved argument when available instead
            // of resolving the `ParsedType` a second time. The redundant
            // resolution is exponential on deeply nested generics (each level
            // re-resolves its arguments), so reusing the probe result is what keeps
            // a nominal-reference instantiation linear.
            let (resolved_ty, argument_had_error) = if let Some(pre) = pre_resolved.and_then(|pre| pre.get(index)) {
                (
                    pre.clone(),
                    pre_resolved_errors
                        .and_then(|errors| errors.get(index))
                        .copied()
                        .unwrap_or(false),
                )
            } else {
                let resolved_argument =
                    resolve_parsed_type(argument.clone(), ctx, resolving, parent_substitution);
                (resolved_argument.ty, resolved_argument.had_error)
            };
            bound_had_error |= argument_had_error;

            if parsed_type_is_placeholder_reference(argument, parent_substitution) {
                substitution.insert_placeholder(parameter.name.clone(), resolved_ty);
            } else {
                constraints_settled = constraints_settled
                    || check_type_argument_constraint(
                        parameter,
                        type_parameters,
                        &resolved_ty,
                        argument_had_error,
                        argument_span,
                        ctx,
                        resolving,
                        parent_substitution,
                        &substitution,
                        declaration_scope,
                    );
                substitution.insert(parameter.name.clone(), resolved_ty);
            }
            if argument_had_error {
                substitution.mark_degraded(&parameter.name);
            }
            continue;
        }

        let Some(default_type) = parameter.default_type.clone() else {
            emit_generic_arity(
                name,
                min_type_argument_count,
                type_parameters.len(),
                name_span,
                ctx,
            );
            return None;
        };

        let mut effective_substitution =
            parent_substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
        effective_substitution
            .extend(substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged));
        // tsc `fillMissingTypeArguments`: a default's forward references (to
        // itself or a later parameter, TS2744) read as the error type.
        for forward in &type_parameters[index..] {
            effective_substitution.insert(forward.name.clone(), Type::ErrorType);
        }

        let default_type_is_placeholder =
            parsed_type_is_placeholder_reference(&default_type, &effective_substitution);
        // A type-parameter default (`T extends X = X`) is authored in the
        // *declaring* module: resolve it under that module's scope and file, not
        // the consumer's, so an imported generic alias/interface binds its
        // defaults even when they name non-exported siblings.
        let resolved_default = if let Some((scope, file_name)) = declaration_scope {
            with_type_declaration_scope(scope, ctx, |ctx| {
                with_file_name(ctx, file_name, |ctx| {
                    resolve_parsed_type(default_type, ctx, resolving, &effective_substitution)
                })
            })
        } else {
            resolve_parsed_type(default_type, ctx, resolving, &effective_substitution)
        };
        bound_had_error |= resolved_default.had_error;

        if default_type_is_placeholder {
            substitution.insert_placeholder(parameter.name.clone(), resolved_default.ty);
        } else {
            substitution.insert(parameter.name.clone(), resolved_default.ty);
        }
        if resolved_default.had_error {
            substitution.mark_degraded(&parameter.name);
        }
    }

    Some(BoundTypeArguments {
        substitution,
        had_error: bound_had_error,
    })
}

pub(crate) fn extend_substitution_with_type_parameters(
    parent_substitution: &TypeParameterSubstitution,
    type_parameters: &[ParsedTypeParameter],
    value_parameters: &[surge_ts_syntax::ParsedFunctionTypeParameter],
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
) -> TypeParameterSubstitution {
    let mut substitution =
        parent_substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);

    for parameter in type_parameters {
        let mut effective_substitution =
            parent_substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
        effective_substitution
            .extend(substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged));

        // A signature's own type parameter is decided at the call site, so a
        // declared default only applies when nothing can infer it. Surge builds
        // this `FunctionType` without call context and drops the type-parameter
        // list, so a default baked into a *parameter* annotation hardens an
        // uninferred generic into a concrete type: `m<T = string>(x: T)` would
        // reject `o.m(42)`, and `f1<Q extends K[] = K[]>(o: Opts<Q>)` would
        // reject a caller's own `Opts<Q>`. Bind the uninferred sentinel there
        // instead — the reading assignability already treats as "could not be
        // inferred" — and keep the default for a parameter no argument mentions
        // (`m<T = string>(): T[]`), where tsc uses it too. Only a defaulted
        // parameter can be hardened, so the scan is gated on there being one.
        let inferable_from_arguments = parameter.default_type.is_some()
            && value_parameters.iter().any(|value_parameter| {
                parsed_type_mentions_type_parameter(&value_parameter.ty, &parameter.name)
            });

        let resolved = parameter.default_type.clone().map(|default_type| {
            resolve_parsed_type(default_type, ctx, resolving, &effective_substitution)
        });

        let ty = match resolved {
            Some(_) if inferable_from_arguments => Type::Unknown,
            Some(resolved) if !resolved.had_error => resolved.ty,
            Some(_) => Type::Unknown,
            // An unconstrained parameter with nothing to default to stands for
            // itself: it relates like the sentinel everywhere, but a generic
            // signature compared with a non-generic one can tell it is bound.
            None if parameter.constraint.is_none() => {
                Type::TypeParameter(surge_ts_types::TypeParameterType {
                    name: parameter.name.as_str().into(),
                    owner: 0,
                })
            }
            None => Type::Unknown,
        };

        if let Some(default_type) = parameter.default_type.as_ref() {
            if parsed_type_is_placeholder_reference(default_type, &effective_substitution) {
                substitution.insert_placeholder(parameter.name.clone(), ty);
                continue;
            }
        }

        substitution.insert(parameter.name.clone(), ty);
    }

    substitution
}

/// Whether `ty` names the type parameter `name` anywhere within it. A syntactic
/// over-approximation: a nested signature that shadows `name` still counts, which
/// only ever moves a binding toward the permissive uninferred sentinel.
fn parsed_type_mentions_type_parameter(ty: &ParsedType, name: &str) -> bool {
    match ty {
        ParsedType::Named(named) => {
            named.name == name
                || named
                    .type_arguments
                    .iter()
                    .any(|argument| parsed_type_mentions_type_parameter(argument, name))
        }
        ParsedType::Array(inner) | ParsedType::KeyOf(inner) => {
            parsed_type_mentions_type_parameter(inner, name)
        }
        ParsedType::Union(members)
        | ParsedType::Intersection(members)
        | ParsedType::Tuple(members) => members
            .iter()
            .any(|member| parsed_type_mentions_type_parameter(member, name)),
        ParsedType::Function(function) => {
            function
                .parameters
                .iter()
                .any(|parameter| parsed_type_mentions_type_parameter(&parameter.ty, name))
                || parsed_type_mentions_type_parameter(&function.return_type, name)
        }
        ParsedType::Object(object) => {
            object
                .properties
                .iter()
                .any(|property| parsed_type_mentions_type_parameter(&property.ty, name))
                || object
                    .construct_signature
                    .as_deref()
                    .into_iter()
                    .chain(object.call_signature.as_deref())
                    .any(|signature| {
                        signature
                            .parameters
                            .iter()
                            .any(|parameter| {
                                parsed_type_mentions_type_parameter(&parameter.ty, name)
                            })
                            || parsed_type_mentions_type_parameter(&signature.return_type, name)
                    })
        }
        ParsedType::IndexedAccess(indexed) => {
            parsed_type_mentions_type_parameter(&indexed.object_type, name)
                || parsed_type_mentions_type_parameter(&indexed.index_type, name)
        }
        ParsedType::Mapped(mapped) => {
            parsed_type_mentions_type_parameter(&mapped.constraint, name)
                || parsed_type_mentions_type_parameter(&mapped.value_type, name)
        }
        ParsedType::Conditional(conditional) => {
            parsed_type_mentions_type_parameter(&conditional.check_type, name)
                || parsed_type_mentions_type_parameter(&conditional.extends_type, name)
                || parsed_type_mentions_type_parameter(&conditional.true_type, name)
                || parsed_type_mentions_type_parameter(&conditional.false_type, name)
        }
        ParsedType::TemplateLiteral(template) => template
            .interpolations
            .iter()
            .any(|part| parsed_type_mentions_type_parameter(part, name)),
        ParsedType::Predicate(predicate) => predicate
            .ty
            .as_ref()
            .is_some_and(|ty| parsed_type_mentions_type_parameter(ty, name)),
        _ => false,
    }
}

pub(crate) fn parsed_type_is_placeholder_reference(
    parsed_type: &ParsedType,
    substitution: &TypeParameterSubstitution,
) -> bool {
    matches!(
        parsed_type,
        ParsedType::Named(named_type) if substitution.is_placeholder(&named_type.name)
    )
}

pub(crate) fn parsed_type_placeholder_name<'a>(
    parsed_type: &'a ParsedType,
    substitution: &TypeParameterSubstitution,
) -> Option<&'a str> {
    match parsed_type {
        ParsedType::Named(named_type) if substitution.is_placeholder(&named_type.name) => {
            Some(named_type.name.as_str())
        }
        _ => None,
    }
}

pub(crate) fn is_concrete_substituted_named_reference(
    parsed_type: &ParsedType,
    substitution: &TypeParameterSubstitution,
) -> bool {
    matches!(
        parsed_type,
        ParsedType::Named(named_type)
            if substitution
                .get(&named_type.name)
                .is_some()
                && !substitution.is_placeholder(&named_type.name)
    )
}

pub(crate) fn is_concrete_substituted_index_reference(
    parsed_type: &ParsedType,
    substitution: &TypeParameterSubstitution,
) -> bool {
    match parsed_type {
        ParsedType::Named(named_type) => {
            substitution.get(&named_type.name).is_some()
                && !substitution.is_placeholder(&named_type.name)
        }
        ParsedType::KeyOf(inner) => {
            is_concrete_substituted_named_reference(inner.as_ref(), substitution)
        }
        _ => false,
    }
}

/// `Expect<false>` where `Expect<a extends true>`: a written type argument has to
/// satisfy its parameter's constraint, which surge never checked for a type
/// reference (only for a call's explicit `keyof` arguments). Both sides must be
/// settled — an argument or constraint that degraded says nothing about whether
/// the constraint holds — and a constraint naming an earlier parameter resolves
/// under the bindings made so far (`<T, K extends keyof T>`).
#[allow(clippy::too_many_arguments)]
fn check_type_argument_constraint(
    parameter: &ParsedTypeParameter,
    type_parameters: &[ParsedTypeParameter],
    argument: &Type,
    argument_had_error: bool,
    name_span: Option<TextSpan>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    parent_substitution: &TypeParameterSubstitution,
    bound_so_far: &TypeParameterSubstitution,
    declaration_scope: Option<(
        &Option<std::sync::Arc<crate::symbols::TypeDeclarationScope>>,
        &str,
    )>,
) -> bool {
    let Some(constraint) = parameter.constraint.clone() else {
        return false;
    };
    let constraint_for_display = constraint.clone();
    // An object type satisfies no primitive constraint whatever its members,
    // so an argument outside the judgeable domain is still decided against a
    // constraint written as primitives (`Record<Date, …>`). The written form
    // is asked first: telling an object apart peels the argument.
    if argument_had_error
        || !(object_constraint_judgeable(argument)
            || constraint_is_written_primitive(&constraint) && is_object_kind(argument))
    {
        return false;
    }
    // A constraint stated in terms of a *sibling* parameter (`K extends keyof T
    // & string`) is only as right as surge's model of that sibling. It resolves
    // to a literal union, which passes the judgeability test below while saying
    // nothing about whether the argument is really out of constraint: drizzle's
    // `PgSelectWithout<T, TDynamic, K extends keyof T & string>` reported its own
    // method-name union as violating `keyof T` in five files, because `T`'s keys
    // came back as an unrelated shape. The assertion idiom this check exists for
    // (`Expect<a extends true>`) never names a sibling. Built-in `Pick` checks
    // its key constraint separately at the written reference.
    // A bare earlier sibling (`U extends T`) is exactly the argument already
    // bound to it, so it is judged like any written type.
    let own_index = type_parameters.iter().position(|sibling| sibling.name == parameter.name);
    let bare_earlier_sibling = matches!(
        &constraint,
        ParsedType::Named(named) if named.type_arguments.is_empty()
            && type_parameters
                .iter()
                .position(|sibling| sibling.name == named.name)
                .is_some_and(|index| own_index.is_some_and(|own| index < own))
    );
    if !bare_earlier_sibling && constraint_names_a_sibling(&constraint, type_parameters) {
        return false;
    }
    // surge's `any` also stands for a type it could not infer, and against a
    // sibling's argument that reads as a violation tsc would not see.
    if bare_earlier_sibling && matches!(argument, Type::Any) {
        return false;
    }

    let mut effective = parent_substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
    effective.extend(bound_so_far.clone_with_reason(TypeCopyReason::SubstitutionChanged));
    // A constraint is authored in the *declaring* module and names its siblings
    // bare, exactly like a default: resolving it under the consumer's scope
    // reports every such sibling as an unknown name. The resolution is also
    // speculative — it exists only to judge the argument — so anything it emits
    // on the way is rolled back.
    let diagnostics_before = ctx.diagnostics().len();
    let resolved_constraint = if is_written_keyof_any(&constraint) {
        // `getIndexType(any)`. surge resolves `keyof any` to the sentinel, its
        // `Type::Any` also standing in for types it could not model; the
        // written keyword is the real `any`.
        ResolvedType {
            ty: union_type(vec![Type::String, Type::Number, Type::Symbol]),
            had_error: false,
        }
    } else if let Some((scope, file_name)) = declaration_scope {
        with_type_declaration_scope(scope, ctx, |ctx| {
            with_file_name(ctx, file_name, |ctx| {
                resolve_parsed_type(constraint, ctx, resolving, &effective)
            })
        })
    } else {
        resolve_parsed_type(constraint, ctx, resolving, &effective)
    };
    ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
    if resolved_constraint.had_error
        || !object_constraint_judgeable(&resolved_constraint.ty)
        || !object_constraint_judgeable(argument) && !is_primitive_kind(&resolved_constraint.ty)
    {
        return false;
    }

    // A primitive relates to an object constraint through its apparent type,
    // which a global augmentation may extend (`interface Number extends …`).
    if !constraint_judgeable(&resolved_constraint.ty) && !is_object_kind(argument) {
        return false;
    }
    if surge_ts_types::is_assignable_to(argument, &resolved_constraint.ty) {
        return false;
    }
    let constraint_name = written_constraint_display(&constraint_for_display, &effective)
        .unwrap_or_else(|| resolved_constraint.ty.name().to_string());
    // `reportRelationError` names a literal source by its base type when the
    // constraint could not hold it: `Uppercase<42>` reports 'number'.
    let argument_name = crate::checks::expr::source_display_name(argument, &resolved_constraint.ty);
    // relater.go drops the TS2344 head when a missing-property report for
    // the same pair is beneath it.
    if let Some(span) = name_span
        && let Some(diagnostic) = crate::checks::expr::missing_properties_report(
            argument,
            &resolved_constraint.ty,
            &argument_name,
            &constraint_name,
            &ctx.file_name.clone(),
        )
    {
        ctx.push_utility_diagnostic_once(diagnostic.with_span(crate::context::convert_span(span)));
        return true;
    }
    crate::infer::types::diagnostics::emit_type_argument_constraint(
        &argument_name,
        &constraint_name,
        name_span,
        ctx,
    );
    true
}

/// [`constraint_judgeable`] widened to plain object types whose every member
/// is itself judgeable (`interface Derived { foo: string; bar: string }`), for
/// the type-argument check alone.
fn object_constraint_judgeable(ty: &Type) -> bool {
    fn walk(ty: &Type, depth: usize) -> bool {
        if constraint_judgeable(ty) {
            return true;
        }
        if depth > 4 {
            return false;
        }
        match ty {
            Type::Object(object) => {
                !object.is_intersection
                    && !object.non_primitive
                    && !object.synthetic_open_index
                    && object.call_signature().is_none()
                    && object.construct_signature().is_none()
                    && object.string_index_type.as_deref().is_none_or(|index| walk(index, depth + 1))
                    && object.number_index_type.as_deref().is_none_or(|index| walk(index, depth + 1))
                    && object.properties.values().all(|property| walk(&property.ty, depth + 1))
            }
            Type::Reference(_) => walk(&ty.peeled(), depth + 1),
            Type::Array(element) => walk(element, depth + 1),
            Type::Union(union) => union.types().iter().all(|member| walk(member, depth + 1)),
            _ => false,
        }
    }
    walk(ty, 0)
}

/// `checkTypeReferenceNode` relates a written reference's arguments to their
/// parameters' constraints wherever the reference appears. A library
/// declaration's instantiation is deferred to a lazy reference or served from a
/// program-wide cache, and neither binds the arguments at the reference, so
/// `ReturnType<string>` and `Uppercase<42>` were never related. Only a
/// reference written in a checked file is related: one re-resolved under an
/// instantiating substitution is not the node tsc checks, and tsc reports
/// nothing for it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_written_type_argument_constraints(
    type_parameters: &[ParsedTypeParameter],
    written: &[ParsedType],
    arguments: &[Type],
    name_span: Option<TextSpan>,
    substitution: &TypeParameterSubstitution,
    declaration_scope: (
        &Option<std::sync::Arc<crate::symbols::TypeDeclarationScope>>,
        &str,
    ),
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
) {
    if name_span.is_none()
        || ctx.is_library_scoped_file(&ctx.file_name)
        || written
            .iter()
            .any(|argument| names_instantiated_parameter(argument, substitution))
    {
        return;
    }
    let unbound = TypeParameterSubstitution::new();
    let mut bound = TypeParameterSubstitution::new();
    for (parameter, argument) in type_parameters.iter().zip(arguments) {
        if check_type_argument_constraint(
            parameter,
            type_parameters,
            argument,
            false,
            name_span,
            ctx,
            resolving,
            &unbound,
            &bound,
            Some(declaration_scope),
        ) {
            return;
        }
        bound.insert(parameter.name.clone(), argument.clone());
    }
}

/// Whether a written type argument names a parameter the substitution binds to
/// a type rather than to itself: the reference is being re-resolved as part of
/// an instantiation.
pub(super) fn names_instantiated_parameter(argument: &ParsedType, substitution: &TypeParameterSubstitution) -> bool {
    substitution.iter().next().is_some()
        && super::indexed_access::mentions_type_parameter(argument, &|name: &str| {
            substitution.get(name).is_some() && !substitution.is_placeholder(name)
        })
}

/// The constraint as written, for the diagnostic. `keyof T` is spelled out
/// rather than expanded because that is how tsc names it and how `Pick`'s own
/// check in `utility.rs` already reports it — two spellings of one constraint
/// are two dedup keys, and the same violation was reported twice. Kept local
/// rather than added to `parsed_type_display`, whose output also prefixes
/// rendered function types.
fn written_constraint_display(
    constraint: &ParsedType,
    substitution: &TypeParameterSubstitution,
) -> Option<String> {
    match constraint {
        // `keyof any` is `string | number | symbol` itself, and tsc names it so.
        ParsedType::KeyOf(_) if is_written_keyof_any(constraint) => None,
        ParsedType::KeyOf(inner) => Some(format!(
            "keyof {}",
            written_constraint_display(inner, substitution)?
        )),
        // A constraint names the declaration's *own* parameters (`Pick<T, K
        // extends keyof T>`), and tsc renders them substituted — `keyof User`,
        // not `keyof T`.
        ParsedType::Named(named) if named.type_arguments.is_empty() => Some(
            substitution
                .get(&named.name)
                .map(|bound| bound.name().to_string())
                .unwrap_or_else(|| named.name.clone()),
        ),
        other => crate::driver::parsed_type_display(other),
    }
}

/// Whether `constraint` mentions one of the declaration's own type parameters.
/// Anything this cannot look inside counts as mentioning one: the check is worth
/// having only where it is certain, and a missed report costs less than a false
/// one.
pub(crate) fn constraint_names_a_sibling(
    constraint: &ParsedType,
    siblings: &[ParsedTypeParameter],
) -> bool {
    let names_sibling = |name: &str| siblings.iter().any(|sibling| sibling.name == name);
    match constraint {
        ParsedType::String
        | ParsedType::Number
        | ParsedType::Boolean
        | ParsedType::BigInt
        | ParsedType::Symbol
        | ParsedType::Undefined
        | ParsedType::Void
        | ParsedType::Any
        | ParsedType::Unknown
        | ParsedType::UnknownKeyword
        | ParsedType::Never
        | ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_) => false,
        ParsedType::Named(named) => {
            names_sibling(&named.name)
                || named
                    .type_arguments
                    .iter()
                    .any(|argument| constraint_names_a_sibling(argument, siblings))
        }
        ParsedType::Array(inner) | ParsedType::KeyOf(inner) => {
            constraint_names_a_sibling(inner, siblings)
        }
        ParsedType::Union(members)
        | ParsedType::Intersection(members)
        | ParsedType::Tuple(members) => members
            .iter()
            .any(|member| constraint_names_a_sibling(member, siblings)),
        // The `object` keyword, which lowers to a memberless object type.
        ParsedType::Object(object) if object.non_primitive => false,
        ParsedType::Function(function) => signature_names_a_sibling(function, siblings),
        // A type made only of call and construct signatures
        // (`abstract new (...args: any) => any`).
        ParsedType::Object(object)
            if object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature_overloads.is_empty()
                && (object.call_signature.is_some() || object.construct_signature.is_some()) =>
        {
            object
                .call_signature
                .iter()
                .chain(&object.construct_signature)
                .any(|signature| signature_names_a_sibling(signature, siblings))
        }
        ParsedType::Object(object)
            if object.call_signature.is_none()
                && object.construct_signature.is_none()
                && object.call_signature_overloads.is_empty()
                && object.construct_signature_overloads.is_empty() =>
        {
            object.properties.iter().any(|property| {
                constraint_names_a_sibling(&property.ty, siblings)
                    || property.write_ty.as_ref().is_some_and(|ty| constraint_names_a_sibling(ty, siblings))
            }) || object
                .string_index_type
                .iter()
                .chain(&object.number_index_type)
                .any(|index| constraint_names_a_sibling(index, siblings))
        }
        _ => true,
    }
}

/// A generic signature binds names of its own, which a sibling may share, so
/// it counts as naming one.
fn signature_names_a_sibling(
    function: &surge_ts_syntax::ParsedFunctionType,
    siblings: &[ParsedTypeParameter],
) -> bool {
    !function.type_parameters.is_empty()
        || function
            .parameters
            .iter()
            .any(|parameter| constraint_names_a_sibling(&parameter.ty, siblings))
        || constraint_names_a_sibling(&function.return_type, siblings)
}

/// A type whose constraint relationship surge can judge without leaning on
/// structure it may model wrongly: primitives and literals, `null`,
/// `undefined`, `void`, `never`, `any`, `unknown`, the `object` keyword, and
/// arrays, tuples and plain signatures built from them. Every structural gap
/// surge still has (an interface that should satisfy `object`, a lazy reference
/// whose shape is not forced) would otherwise surface here as a false `TS2344`
/// on code that is fine, and this check has no way to tell that from a real
/// violation. The global `Function` interface is the one named type admitted:
/// it declares no call signature, and that alone decides how it relates to
/// every other type in this domain.
fn constraint_judgeable(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Null
        | Type::Undefined
        | Type::Void
        | Type::Never
        | Type::Any
        | Type::GenuineUnknown
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Array(element) => constraint_judgeable(element),
        Type::Tuple(elements) => elements.iter().all(constraint_judgeable),
        Type::OpenTuple(tuple) => tuple
            .leading
            .iter()
            .chain(std::iter::once(tuple.rest.as_ref()))
            .chain(&tuple.trailing)
            .all(constraint_judgeable),
        Type::Function(function) => signature_judgeable(function),
        // `object`, or a type made only of call and construct signatures
        // (`abstract new (...args: any) => any`).
        Type::Object(object) => {
            let has_signature =
                object.call_signature().is_some() || object.construct_signature().is_some();
            !object.is_intersection
                && object.properties.is_empty()
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && (object.non_primitive && !has_signature
                    || !object.non_primitive
                        && has_signature
                        && object.call_signature().is_none_or(signature_judgeable)
                        && object.construct_signature().is_none_or(signature_judgeable))
        }
        Type::Union(union) => union.types().iter().all(constraint_judgeable),
        Type::Reference(_) => surge_ts_types::is_global_function_interface(ty),
        _ => false,
    }
}

/// A signature over the judgeable domain. A generic one binds type parameters
/// of its own, and an overload fold stands for several signatures; neither
/// relates as the one signature surge holds.
fn signature_judgeable(function: &surge_ts_types::FunctionType) -> bool {
    function.type_parameter_head().is_none()
        && function.overloads().is_none()
        && function.parameters().iter().all(constraint_judgeable)
        && constraint_judgeable(function.return_type())
}

/// [`constraint_judgeable`] for a relation that may involve the type variables
/// of the declaration being checked: a variable relates through its constraint
/// (`unknown` when it has none), so it is judged when that constraint is.
pub(crate) fn judgeable_through_type_variable(ty: &Type) -> bool {
    match ty {
        Type::TypeParameter(parameter) => {
            match surge_ts_types::type_variable::active_constraint(parameter) {
                Some(None) => true,
                Some(Some(constraint)) => constraint_judgeable(&constraint),
                None => false,
            }
        }
        other => constraint_judgeable(other),
    }
}

/// Whether surge decides `argument` against the constraint `constraint`
/// exactly: both are judgeable, or an object type meets a primitive constraint.
pub(crate) fn constraint_relation_decidable(argument: &Type, constraint: &Type) -> bool {
    constraint_judgeable(constraint)
        && (constraint_judgeable(argument)
            || is_primitive_kind(constraint) && is_object_kind(argument))
}

/// A constraint written as primitives and literals alone (`string`, `keyof
/// any`, `'a' | 1`), read off the syntax before anything is resolved.
fn constraint_is_written_primitive(constraint: &ParsedType) -> bool {
    match constraint {
        ParsedType::String
        | ParsedType::Number
        | ParsedType::Boolean
        | ParsedType::BigInt
        | ParsedType::Symbol
        | ParsedType::Null
        | ParsedType::Undefined
        | ParsedType::Void
        | ParsedType::Never
        | ParsedType::StringLiteral(_)
        | ParsedType::NumberLiteral(_)
        | ParsedType::BooleanLiteral(_) => true,
        ParsedType::KeyOf(_) => is_written_keyof_any(constraint),
        ParsedType::Union(members) => members.iter().all(constraint_is_written_primitive),
        _ => false,
    }
}

fn is_written_keyof_any(constraint: &ParsedType) -> bool {
    matches!(constraint, ParsedType::KeyOf(inner) if matches!(inner.as_ref(), ParsedType::Any))
}

/// A resolved type every member of which is a primitive or a literal.
fn is_primitive_kind(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Null
        | Type::Undefined
        | Type::Void
        | Type::Never
        | Type::StringLiteral(_)
        | Type::NumberLiteral(_)
        | Type::BooleanLiteral(_) => true,
        Type::Union(union) => union.types().iter().all(is_primitive_kind),
        _ => false,
    }
}

/// An object type whatever its members, as relater.go's `isObjectType` (or
/// the `object` keyword) sees it. A branded intersection keeps its primitive
/// operand, and an enum, a unique symbol, a template literal or a string
/// mapping is a reference that is no object.
fn is_object_kind(ty: &Type) -> bool {
    match ty {
        Type::Function(_) | Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_) => true,
        Type::Object(object) => !object.is_intersection,
        Type::Reference(reference) => {
            reference.is_readonly_array()
                || reference.enum_owner.is_none()
                    && !reference.is_unique_symbol()
                    && !surge_ts_types::is_template_literal_type(ty)
                    && surge_ts_types::string_mapping_parts(ty).is_none()
                    && is_object_kind(&ty.peeled())
        }
        _ => false,
    }
}
