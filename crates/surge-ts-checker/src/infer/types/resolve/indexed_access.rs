use super::*;

use surge_ts_syntax::{ParsedIndexedAccessType, ParsedTupleElement, TextSpan};

use crate::program::{
    record_generic_indexed_access_attempt, record_generic_indexed_access_invalid_key,
    record_generic_indexed_access_substituted_key,
    record_generic_indexed_access_substituted_receiver, record_generic_indexed_access_success,
    record_generic_indexed_access_unknown_fallback,
};

trait ParsedTypeSpan {
    fn span(&self) -> Option<TextSpan>;
}

impl ParsedTypeSpan for ParsedType {
    fn span(&self) -> Option<TextSpan> {
        match self {
            ParsedType::Named(named_type) => named_type.span,
            ParsedType::TypeOf(type_of) => type_of.name_span,
            ParsedType::IndexedAccess(indexed_access) => indexed_access.span,
            ParsedType::Mapped(mapped) => mapped.key_span.or(mapped.span),
            _ => None,
        }
    }
}

/// Selects the type of `index` from `object` without emitting diagnostics or
/// recording cascade errors. Used when the receiver already errored but its
/// structural shape is still usable, so the requested property can be selected
/// without cascading a fresh missing-property diagnostic. Returns `None` when
/// the receiver is not an indexable structure or the key is not present.
fn select_indexed_property_no_cascade(object: &Type, index: &Type) -> Option<Type> {
    match (object, index) {
        (Type::Object(object_type), Type::StringLiteral(key)) => {
            object_type.get_property_access_type(key)
        }
        (Type::Object(object_type), Type::String) => {
            object_type.string_index_type.as_deref().cloned()
        }
        (Type::Object(object_type), Type::Union(union_ty)) => {
            let mut types = Vec::new();
            for key_ty in union_ty.types() {
                let Type::StringLiteral(key) = key_ty else {
                    return None;
                };
                types.push(object_type.get_property_access_type(key)?);
            }
            Some(union_type(types))
        }
        (Type::Tuple(elements), Type::NumberLiteral(num)) => {
            let index = num.value.parse::<usize>().ok()?;
            elements.get(index).cloned()
        }
        (Type::Array(element_type), Type::Number) => Some(*element_type.clone()),
        (Type::Tuple(elements), Type::Number) => Some(union_type(elements.clone())),
        (Type::Tuple(elements), Type::StringLiteral(key)) if key == "length" => Some(
            Type::NumberLiteral(surge_ts_types::NumberLiteralType { value: elements.len().to_string() }),
        ),
        (Type::OpenTuple(_), Type::StringLiteral(key)) if key == "length" => Some(Type::Number),
        (Type::OpenTuple(open), Type::Number | Type::NumberLiteral(_)) => Some(open.element_union()),
        _ => None,
    }
}

/// Whether the written index is a key of `object_name` on its face: `T[keyof T]`
/// or an intersection that includes `keyof T` (`Ms['length' & keyof Ms]` —
/// zustand's `Mutate`), which is assignable to `keyof T` and so a valid key.
fn index_narrows_to_keyof(index: &ParsedType, object_name: &str) -> bool {
    match index {
        ParsedType::KeyOf(inner) => matches!(
            inner.as_ref(),
            ParsedType::Named(named_type) if named_type.name == object_name
        ),
        ParsedType::Intersection(members) => members
            .iter()
            .any(|member| index_narrows_to_keyof(member, object_name)),
        _ => false,
    }
}

/// Whether `parsed` refers by name to a type parameter `bound` accepts.
pub(super) fn mentions_type_parameter(parsed: &ParsedType, bound: &dyn Fn(&str) -> bool) -> bool {
    let mentions = |ty: &ParsedType| mentions_type_parameter(ty, bound);
    match parsed {
        ParsedType::Named(named) => bound(&named.name) || named.type_arguments.iter().any(mentions),
        ParsedType::Array(inner) | ParsedType::KeyOf(inner) | ParsedType::Readonly(inner) => {
            mentions(inner)
        }
        ParsedType::Tuple(members) | ParsedType::Union(members) | ParsedType::Intersection(members) => {
            members.iter().any(mentions)
        }
        ParsedType::VariadicTuple(elements) => elements.iter().any(|element| match element {
            ParsedTupleElement::Fixed(ty) | ParsedTupleElement::Rest(ty, _) => mentions(ty),
        }),
        ParsedType::IndexedAccess(indexed) => {
            mentions(&indexed.object_type) || mentions(&indexed.index_type)
        }
        ParsedType::Conditional(conditional) => {
            mentions(&conditional.check_type)
                || mentions(&conditional.extends_type)
                || mentions(&conditional.true_type)
                || mentions(&conditional.false_type)
        }
        ParsedType::Mapped(mapped) => {
            mentions(&mapped.constraint)
                || mentions(&mapped.value_type)
                || mapped.name_type.as_deref().is_some_and(mentions)
        }
        ParsedType::TemplateLiteral(template) => template.interpolations.iter().any(mentions),
        ParsedType::Function(function) => {
            function.parameters.iter().any(|parameter| mentions(&parameter.ty))
                || mentions(&function.return_type)
        }
        ParsedType::Object(object) => {
            object.properties.iter().any(|property| mentions(&property.ty))
                || object.string_index_type.as_deref().is_some_and(mentions)
                || object.number_index_type.as_deref().is_some_and(mentions)
                || object
                    .call_signature
                    .as_deref()
                    .into_iter()
                    .chain(object.construct_signature.as_deref())
                    .any(|signature| {
                        signature.parameters.iter().any(|parameter| mentions(&parameter.ty))
                            || mentions(&signature.return_type)
                    })
        }
        _ => false,
    }
}

/// Whether a literal of `index_name`'s written constraint (`'a' | 'b'`) names
/// no property of `object`, when `object` is a closed object type: one with no
/// index signature and nothing surge could not model.
fn literal_constraint_misses_key_of(index_name: &str, object: &Type, ctx: &CheckerContext) -> bool {
    let Some(constraint) = ctx.type_parameter_constraint(index_name) else {
        return false;
    };
    let literals: Vec<&str> = match constraint {
        ParsedType::StringLiteral(literal) => vec![literal.as_str()],
        ParsedType::Union(members) => {
            let Some(literals) = members
                .iter()
                .map(|member| match member {
                    ParsedType::StringLiteral(literal) => Some(literal.as_str()),
                    _ => None,
                })
                .collect::<Option<_>>()
            else {
                return false;
            };
            literals
        }
        _ => return false,
    };
    let Type::Object(object_type) = object else {
        return false;
    };
    if object_type.string_index_type.is_some()
        || object_type.number_index_type.is_some()
        || object_type.keyed_indexes.is_some()
        || object_type.synthetic_open_index
        || object_type.is_intersection
        || object.is_unmodelled()
        || crate::checks::function::type_contains_degradation(object)
    {
        return false;
    }
    literals.into_iter().any(|literal| !object_type.properties.contains_key(literal))
}

pub(super) fn resolve_indexed_access_type(
    indexed_access: ParsedIndexedAccessType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    record_generic_indexed_access_attempt();
    let object_type_for_placeholder = indexed_access.object_type.clone();
    let object_placeholder_name =
        parsed_type_placeholder_name(object_type_for_placeholder.as_ref(), substitution);
    let index_placeholder_name =
        parsed_type_placeholder_name(indexed_access.index_type.as_ref(), substitution);
    let object_is_concrete_substitution =
        is_concrete_substituted_named_reference(object_type_for_placeholder.as_ref(), substitution);
    let index_is_concrete_substitution =
        is_concrete_substituted_index_reference(indexed_access.index_type.as_ref(), substitution);
    let generic_indexed_access = object_placeholder_name.is_some()
        || index_placeholder_name.is_some()
        || object_is_concrete_substitution
        || index_is_concrete_substitution;
    // tsc decides an indexed access's errors only while resolving the written
    // node (`accessNode`), where an access naming a type parameter is deferred
    // as generic; its instantiation resolves with no node at all. Surge
    // re-resolves the node under the instantiating substitution instead, so a
    // report there would describe the instantiation, not the source — the
    // generated hey-api clients re-validate `T['baseUrl']` once per candidate
    // substitution.
    let bound = |name: &str| substitution.get(name).is_some();
    let replaced = |name: &str| bound(name) && !substitution.is_placeholder(name);
    let mentions_bound = mentions_type_parameter(object_type_for_placeholder.as_ref(), &bound)
        || mentions_type_parameter(indexed_access.index_type.as_ref(), &bound);
    let instantiating = object_is_concrete_substitution
        || index_is_concrete_substitution
        || mentions_type_parameter(object_type_for_placeholder.as_ref(), &replaced)
        || mentions_type_parameter(indexed_access.index_type.as_ref(), &replaced);
    // A key parameter bound to a type rather than to itself — a mapped type's
    // key, a method's constrained parameter — is an instantiation of the key
    // `checkIndexedAccessIndexType` validates, not the key itself.
    let index_is_instantiated =
        mentions_type_parameter(indexed_access.index_type.as_ref(), &|name: &str| {
            replaced(name) && !matches!(substitution.get(name), Some(Type::TypeParameter(_)))
        });
    let index_is_keyof_same_placeholder = object_placeholder_name
        .as_deref()
        .is_some_and(|object_name| {
            index_narrows_to_keyof(indexed_access.index_type.as_ref(), object_name)
        });
    // `K extends keyof T` makes the generic `T[K]` a valid index even though
    // neither side is concrete yet, so it must not cascade into TS2536.
    let index_constraint_satisfies_object = match (
        object_placeholder_name.as_deref(),
        index_placeholder_name.as_deref(),
    ) {
        (Some(object_name), Some(index_name)) => {
            ctx.type_parameter_keyof_constraint_target(index_name) == Some(object_name)
        }
        _ => false,
    };
    let index_is_valid_generic_key =
        index_is_keyof_same_placeholder || index_constraint_satisfies_object;

    // An index access through a *constrained* type parameter (`T extends …`,
    // `K extends Key`, `strict extends Boolean`, …) is validated by tsc against
    // that constraint. We do not fully resolve those (often library-generated)
    // constraints, so verifying the key here would only ever produce false
    // `TS2536`/`TS2538`s. An unconstrained `T[K]` is still a genuine error and
    // is left to the checks below.
    let involves_constrained_type_parameter = object_placeholder_name
        .as_deref()
        .is_some_and(|name| ctx.type_parameter_has_constraint(name))
        || index_placeholder_name
            .as_deref()
            .is_some_and(|name| ctx.type_parameter_has_constraint(name));

    if object_is_concrete_substitution {
        record_generic_indexed_access_substituted_receiver();
    }
    if index_is_concrete_substitution {
        record_generic_indexed_access_substituted_key();
    }

    let resolved_object =
        resolve_parsed_type(*indexed_access.object_type, ctx, resolving, substitution);
    // Peel a nominal reference receiver (`User["id"]`) to its structural object so
    // the index lookup below reads its properties instead of failing to match.
    let resolved_object = ResolvedType {
        ty: crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::IndexedAccess,
            || resolved_object.ty.peeled(),
        ),
        had_error: resolved_object.had_error,
    };

    let resolved_index = resolve_parsed_type(
        *indexed_access.index_type.clone(),
        ctx,
        resolving,
        substitution,
    );

    if resolved_object.had_error {
        if generic_indexed_access {
            record_generic_indexed_access_unknown_fallback();
        }
        // The receiver shape is known but one of its inner property types could
        // not be resolved (e.g. an imported alias whose body references a lib
        // type unavailable in the declaring module's scope). Still select the
        // requested property so downstream code sees the right type, without
        // emitting a fresh diagnostic from a receiver that already errored. The
        // selected property is a legitimate type, so it is returned clean and
        // participates normally in narrowing. A truly unresolved receiver
        // (Unknown) has no selectable property and stays no-cascade as Unknown.
        if let Some(selected) =
            select_indexed_property_no_cascade(&resolved_object.ty, &resolved_index.ty)
        {
            return ResolvedType {
                ty: selected,
                had_error: false,
            };
        }
        return ResolvedType {
            ty: Type::Unknown,
            had_error: true,
        };
    }

    // The key parameter of a generic mapped type indexes by its constraint,
    // which the mapped type's own declaration check validates; here the
    // access is simply deferred (`shouldDeferIndexedAccessType`).
    if let Type::TypeParameter(index) = &resolved_index.ty
        && matches!(
            surge_ts_types::type_variable::deferred_type(index),
            Some(surge_ts_types::type_variable::DeferredType::MappedKey { .. })
        )
        && let Some(deferred) =
            surge_ts_types::type_variable::indexed_access_variable(&resolved_object.ty, &resolved_index.ty)
    {
        return ResolvedType {
            ty: deferred,
            had_error: false,
        };
    }

    // `getIndexedAccessTypeOrUndefined`: a literal key naming a private or
    // protected member of a type parameter's class constraint is not in
    // `keyof T`, and tsc says why (TS4105) rather than reporting the index.
    if let Some(object_name) = object_placeholder_name.as_deref()
        && let Type::StringLiteral(key) = &resolved_index.ty
        && let Some(ParsedType::Named(constraint)) = ctx.type_parameter_constraint(object_name)
        && let Some(crate::symbols::TypeDeclarationInfo::Interface(class)) =
            ctx.lookup_type_declaration(&constraint.name)
        && class.is_class_instance
        && crate::checks::expr::restricted_member_owner(&class.clone(), key, false, false, ctx).is_some()
    {
        let mut diagnostic = Diagnostic::ts4105(key, ctx.file_name.clone());
        if let Some(span) = indexed_access.span {
            diagnostic = diagnostic.with_span(convert_span(span));
        }
        ctx.push(diagnostic);
        if generic_indexed_access {
            record_generic_indexed_access_invalid_key();
        }
        return ResolvedType {
            ty: Type::Unknown,
            had_error: true,
        };
    }

    // tsc defers an indexed access type whose object is a type variable
    // (`shouldDeferIndexedAccessType`): `T[K]` in a generic body is a type of
    // its own, related through its constraint rather than resolved here. An
    // instantiation re-resolves the node with the variables substituted, which
    // is no longer the body's own access.
    let deferred_access = || {
        (!instantiating)
            .then(|| {
                surge_ts_types::type_variable::indexed_access_variable(&resolved_object.ty, &resolved_index.ty)
            })
            .flatten()
    };
    // `checkIndexedAccessIndexType` for the one constrained key surge can
    // answer: a key parameter constrained by written string literals, into a
    // closed object (`T3[K]`, `K extends 'a' | 'b'`, `T3` holding only `a`).
    if !instantiating
        && object_placeholder_name.is_none()
        && let Some(index_name) = index_placeholder_name.as_deref()
        && literal_constraint_misses_key_of(index_name, &resolved_object.ty, ctx)
    {
        let object_name = match object_type_for_placeholder.as_ref() {
            ParsedType::Named(named) if named.type_arguments.is_empty() => named.name.clone(),
            _ => resolved_object.ty.name(),
        };
        let mut diagnostic = Diagnostic::ts2536(index_name, &object_name, ctx.file_name.clone());
        if let Some(span) = indexed_access.span {
            diagnostic = diagnostic.with_span(convert_span(span));
        }
        ctx.push(diagnostic);
    }
    if (object_placeholder_name.is_some() && index_is_valid_generic_key)
        || involves_constrained_type_parameter
    {
        if let Some(deferred) = deferred_access() {
            if generic_indexed_access {
                record_generic_indexed_access_success();
            }
            return ResolvedType {
                ty: deferred,
                had_error: false,
            };
        }
        if generic_indexed_access {
            record_generic_indexed_access_unknown_fallback();
        }
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    }
    if object_placeholder_name.is_none()
        && let Some(deferred) = deferred_access()
    {
        return ResolvedType {
            ty: deferred,
            had_error: false,
        };
    }

    // A receiver that *resolved* to `unknown` (e.g. `typeof external` whose
    // value type could not be reconstructed, or a generic alias whose body we do
    // not model) cannot have its index validated, so indexing it degrades to
    // `unknown` rather than a false `TS2536`/`TS2538`. Excluded:
    // - a naked type-parameter receiver (`object_placeholder`): unconstrained
    //   `T[K]` is a genuine error handled below;
    // - an *explicit* `unknown`/`any` keyword receiver (`unknown["x"]`): tsc does
    //   report `TS2339`/`TS2538` there, so it must not be suppressed. The
    //   degrade target `ParsedType::Unknown` (`typeof this`, `intrinsic`) is
    //   not one.
    let object_is_explicit_top_keyword = matches!(
        object_type_for_placeholder.as_ref(),
        ParsedType::UnknownKeyword | ParsedType::Any
    );
    if resolved_object.ty.is_unknown()
        && object_placeholder_name.is_none()
        && !object_is_explicit_top_keyword
    {
        if generic_indexed_access {
            record_generic_indexed_access_unknown_fallback();
        }
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    }

    if !index_is_valid_generic_key
        && (index_placeholder_name.is_some() || object_placeholder_name.is_some())
    {
        // `checkIndexedAccessIndexType` asks whether the index is assignable to
        // `keyof T`: `any` and `never` are, and a key that resolved to surge's
        // sentinel cannot be answered either way.
        if index_is_instantiated
            || (index_placeholder_name.is_none()
                && matches!(
                    resolved_index.ty,
                    Type::Unknown | Type::Any | Type::ErrorType | Type::Never
                ))
        {
            if generic_indexed_access {
                record_generic_indexed_access_unknown_fallback();
            }
            return ResolvedType {
                ty: Type::Unknown,
                had_error: false,
            };
        }
        let index_name = index_placeholder_name
            .map(str::to_string)
            .unwrap_or_else(|| resolved_index.ty.name());
        let object_name = object_placeholder_name
            .map(str::to_string)
            .unwrap_or_else(|| resolved_object.ty.name());
        let mut diagnostic = Diagnostic::ts2536(&index_name, &object_name, ctx.file_name.clone());
        if let Some(span) = indexed_access
            .index_type
            .as_ref()
            .span()
            .or(indexed_access.span)
        {
            diagnostic = diagnostic.with_span(convert_span(span));
        }
        ctx.push(diagnostic);
        if generic_indexed_access {
            record_generic_indexed_access_invalid_key();
        }
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    }

    let receiver_unenumerable = receiver_is_unenumerable(&resolved_object.ty);
    let index = index_view(&resolved_index.ty);
    let index_is_degraded = matches!(index, Type::Unknown | Type::TypeParameter(_));
    let index_has_degraded_member = matches!(&index, Type::Union(union) if union
        .types()
        .iter()
        .any(|member| matches!(member, Type::Unknown | Type::TypeParameter(_))));

    if (index_is_degraded || index_has_degraded_member) && matches!(resolved_object.ty, Type::Any) {
        if generic_indexed_access {
            record_generic_indexed_access_success();
        }
        return ResolvedType {
            ty: Type::Any,
            had_error: false,
        };
    }

    if index_is_degraded {
        if generic_indexed_access {
            record_generic_indexed_access_unknown_fallback();
        }
        if receiver_unenumerable {
            return ResolvedType {
                ty: Type::Unknown,
                had_error: false,
            };
        }
        // In a generic context (an access naming a type parameter) an `unknown`
        // index is a resolution limitation we cannot validate — e.g.
        // `X[keyof T]` where `keyof T` could not be computed, which tsc defers —
        // not the literal `value[unknownKey]` that tsc flags.
        let generic = generic_indexed_access || mentions_bound;
        if !generic && ctx.options.diagnostic_profile != crate::context::DiagnosticProfile::Native {
            let mut diagnostic =
                Diagnostic::ts2538(&resolved_index.ty.name(), ctx.file_name.clone());
            if let Some(span) = indexed_access.index_span.or(indexed_access.span) {
                diagnostic = diagnostic.with_span(convert_span(span));
            }
            ctx.push(diagnostic);
        }
        return ResolvedType {
            ty: Type::Unknown,
            had_error: !generic,
        };
    }

    // Symbol keys are not modelled: the parser keeps a computed member
    // (`[matcher](): R`) under a name no symbol key reaches and reads a
    // `[k: symbol]` signature as a string one, so a type keyed by a symbol has
    // no member table to validate against — a report here would be about
    // surge's own gap, not the source. ts-pattern's `CustomP<…>[matcher]` is
    // the shape.
    if index_has_degraded_member
        || (matches!(index, Type::Symbol) && !matches!(resolved_object.ty, Type::Any))
    {
        if generic_indexed_access {
            record_generic_indexed_access_unknown_fallback();
        }
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    }

    let site = (!instantiating && !receiver_unenumerable).then_some(AccessSite {
        span: indexed_access.index_span.or(indexed_access.span),
    });
    match indexed_access_type_or_undefined(&resolved_object.ty, &resolved_index.ty, site, ctx) {
        Some(ty) => {
            if generic_indexed_access {
                record_generic_indexed_access_success();
            }
            ResolvedType {
                ty,
                had_error: false,
            }
        }
        None => {
            if generic_indexed_access {
                record_generic_indexed_access_unknown_fallback();
            }
            if site.is_some() {
                return ResolvedType {
                    ty: Type::Unknown,
                    had_error: true,
                };
            }
            // With no node tsc answers `unknownType`, a real type: a closed
            // object that lacks the key (`TOptions['transformer']` for a
            // `create({ isServer: true })` call decides tRPC's `transformer`
            // flag through it). A receiver surge could not enumerate keeps the
            // sentinel.
            let ty = match (&resolved_object.ty, &index) {
                (Type::Object(object), Type::StringLiteral(_)) if !object.synthetic_open_index => {
                    Type::GenuineUnknown
                }
                _ => Type::Unknown,
            };
            ResolvedType {
                ty,
                had_error: false,
            }
        }
    }
}

/// Where tsc reports an indexed-access error: the index of the written node
/// (`getIndexNodeForAccessExpression(accessNode)`). Absent while an
/// instantiation re-resolves the access.
#[derive(Clone, Copy)]
struct AccessSite {
    span: Option<TextSpan>,
}

impl AccessSite {
    fn report(self, diagnostic: Diagnostic, ctx: &mut CheckerContext) {
        ctx.push(match self.span {
            Some(span) => diagnostic.with_span(convert_span(span)),
            None => diagnostic,
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexKey {
    String,
    Number,
}

struct IndexInfo {
    key: IndexKey,
    value: Type,
}

/// The key kinds `isTypeAssignableToKind` is asked about.
#[derive(Clone, Copy)]
struct KeyKinds {
    string: bool,
    number: bool,
    symbol: bool,
}

/// `StringLike | NumberLike | ESSymbolLike`.
const ANY_KEY_KIND: KeyKinds = KeyKinds {
    string: true,
    number: true,
    symbol: true,
};

/// `String | Number`.
const STRING_OR_NUMBER_KIND: KeyKinds = KeyKinds {
    string: true,
    number: true,
    symbol: false,
};

/// tsc's `getReducedApparentType`: a lazy reference reads as its structural
/// expansion. A primitive stands for its lib interface as it is —
/// `get_property_access_type` answers that interface's members.
fn apparent_type(ty: &Type) -> Type {
    match ty {
        Type::Reference(_) => crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::IndexedAccess,
            || ty.peeled(),
        ),
        other => other.clone(),
    }
}

/// The index as tsc's type flags read it: an enum member is its literal, while a
/// pattern literal (`` `${number}` ``, `Uppercase<string>`) and a unique symbol
/// keep their own identity.
fn index_view(index: &Type) -> Type {
    match index {
        Type::Reference(reference) if reference.is_unique_symbol() => index.clone(),
        Type::Reference(_) => surge_ts_types::peel_to_pattern_literal(index),
        other => other.clone(),
    }
}

/// A receiver surge could not enumerate: an intersection kept open over an
/// operand it dropped, or a union with a member that degraded. Its reads still
/// resolve, but a key it cannot answer says nothing about the source.
fn receiver_is_unenumerable(receiver: &Type) -> bool {
    match receiver {
        Type::Object(object) => object.synthetic_open_index,
        Type::Union(union) => union.types().iter().any(|member| {
            let member = apparent_type(member);
            member.is_degraded()
                || matches!(member, Type::Any)
                || matches!(&member, Type::Object(object) if object.synthetic_open_index)
        }),
        _ => false,
    }
}

fn is_nullable(index: &Type) -> bool {
    matches!(index, Type::Undefined | Type::Null)
}

/// tsc's `isTypeAssignableToKind` for the key kinds.
fn is_assignable_to_key_kind(index: &Type, kinds: KeyKinds) -> bool {
    let string_like = matches!(index, Type::String | Type::StringLiteral(_))
        || surge_ts_types::is_template_literal_type(index)
        || surge_ts_types::string_mapping_parts(index).is_some();
    let number_like = matches!(index, Type::Number | Type::NumberLiteral(_));
    let symbol_like = matches!(index, Type::Symbol)
        || matches!(index, Type::Reference(reference) if reference.is_unique_symbol());
    if (kinds.string && string_like) || (kinds.number && number_like) || (kinds.symbol && symbol_like)
    {
        return true;
    }
    // `unknown` relates to none of them, where surge's permissive sentinels
    // would say otherwise.
    if index.is_degraded() {
        return false;
    }
    (kinds.number && surge_ts_types::is_assignable_to(index, &Type::Number))
        || (kinds.string && surge_ts_types::is_assignable_to(index, &Type::String))
        || (kinds.symbol && surge_ts_types::is_assignable_to(index, &Type::Symbol))
}

/// tsc's `isNumericLiteralName`: the name is the canonical spelling of a number.
fn is_numeric_literal_name(name: &str) -> bool {
    name.parse::<f64>()
        .is_ok_and(|value| surge_ts_syntax::js_number_to_string(value) == name)
}

/// tsc's `numericStringType`, `` `${number}` ``.
fn is_numeric_string_type(ty: &Type) -> bool {
    surge_ts_types::template_literal_parts(ty).is_some_and(|(texts, types)| {
        texts.iter().all(|text| text.is_empty()) && matches!(types.as_slice(), [Type::Number])
    })
}

/// The position a tuple element property is named by (`"0"`, `"1"`, …).
fn tuple_element_position(name: &str) -> Option<usize> {
    let position = name.parse::<usize>().ok()?;
    (position.to_string() == name).then_some(position)
}

fn is_tuple_type(apparent: &Type) -> bool {
    matches!(apparent, Type::Tuple(_) | Type::OpenTuple(_))
}

/// tsc's `everyType` over the members of an apparent receiver.
fn every_member(apparent: &Type, predicate: impl Fn(&Type) -> bool) -> bool {
    match apparent {
        Type::Union(union) => union
            .types()
            .iter()
            .all(|member| predicate(&apparent_type(member))),
        other => predicate(other),
    }
}

/// The union of a tuple's elements, which is `never` for `[]`.
fn tuple_element_union(elements: &[Type]) -> Type {
    if elements.is_empty() {
        Type::Never
    } else {
        union_type(elements.to_vec())
    }
}

/// tsc's `getRestTypeOfTupleType`: every element from the first variable one on.
fn rest_type_of_tuple(tuple: &Type) -> Option<Type> {
    let Type::OpenTuple(open) = tuple else {
        return None;
    };
    let mut elements = vec![open.rest.as_ref().clone()];
    elements.extend(open.trailing.iter().cloned());
    Some(union_type(elements))
}

/// tsc's `getTupleElementTypeOutOfStartCount`: past its named elements a tuple
/// reads its rest elements, and a fixed tuple reads `undefined`.
fn tuple_element_type_out_of_start_count(apparent: &Type) -> Type {
    match apparent {
        Type::Union(union) => union_type(
            union
                .types()
                .iter()
                .map(|member| tuple_element_type_out_of_start_count(&apparent_type(member)))
                .collect(),
        ),
        tuple => rest_type_of_tuple(tuple).unwrap_or(Type::Undefined),
    }
}

/// tsc's `getIndexInfosOfType` over surge's shapes: an object's declared string
/// and number signatures, the element type of an array or tuple, and `String`'s
/// numeric signature on a string.
fn index_infos(apparent: &Type) -> Vec<IndexInfo> {
    match apparent {
        Type::Object(object) => {
            let mut infos = Vec::new();
            if let Some(value) = object.string_index_type.as_deref() {
                infos.push(IndexInfo {
                    key: IndexKey::String,
                    value: value.clone(),
                });
            }
            if let Some(value) = object.number_index_type.as_deref() {
                infos.push(IndexInfo {
                    key: IndexKey::Number,
                    value: value.clone(),
                });
            } else if object
                .intersected_primitive()
                .is_some_and(|primitive| matches!(primitive, Type::String | Type::StringLiteral(_)))
            {
                infos.push(IndexInfo {
                    key: IndexKey::Number,
                    value: Type::String,
                });
            }
            infos
        }
        Type::Array(element) => vec![IndexInfo {
            key: IndexKey::Number,
            value: element.as_ref().clone(),
        }],
        Type::Tuple(elements) => vec![IndexInfo {
            key: IndexKey::Number,
            value: tuple_element_union(elements),
        }],
        Type::OpenTuple(open) => vec![IndexInfo {
            key: IndexKey::Number,
            value: open.element_union(),
        }],
        Type::String | Type::StringLiteral(_) => vec![IndexInfo {
            key: IndexKey::Number,
            value: Type::String,
        }],
        Type::Union(union) => union_index_infos(union.types()),
        _ => Vec::new(),
    }
}

/// tsc's `getUnionIndexInfos`: a signature every member declares, reading the
/// union of their value types.
fn union_index_infos(members: &[Type]) -> Vec<IndexInfo> {
    let members: Vec<Type> = members.iter().map(apparent_type).collect();
    let Some(first) = members.first() else {
        return Vec::new();
    };
    index_infos(first)
        .into_iter()
        .filter_map(|info| {
            let values = members
                .iter()
                .map(|member| index_info_of_type(member, info.key))
                .collect::<Option<Vec<_>>>()?;
            Some(IndexInfo {
                key: info.key,
                value: union_type(values),
            })
        })
        .collect()
}

/// tsc's `getIndexInfoOfType`: the signature declared for exactly `key`.
fn index_info_of_type(apparent: &Type, key: IndexKey) -> Option<Type> {
    index_infos(apparent)
        .into_iter()
        .find(|info| info.key == key)
        .map(|info| info.value)
}

/// tsc's `isApplicableIndexType`: a string signature answers numeric keys too,
/// and a number signature answers `` `${number}` `` and numeric string literals.
fn is_applicable_index_type(source: &Type, key: IndexKey) -> bool {
    match key {
        IndexKey::String => {
            surge_ts_types::is_assignable_to(source, &Type::String)
                || surge_ts_types::is_assignable_to(source, &Type::Number)
        }
        IndexKey::Number => {
            surge_ts_types::is_assignable_to(source, &Type::Number)
                || is_numeric_string_type(source)
                || matches!(source, Type::StringLiteral(value) if is_numeric_literal_name(value))
        }
    }
}

/// tsc's `findApplicableIndexInfo`: the string signature answers only when no
/// other one applies.
fn applicable_index_info(infos: Vec<IndexInfo>, key: &Type) -> Option<IndexInfo> {
    let mut string_info = None;
    for info in infos {
        match info.key {
            IndexKey::String => string_info = Some(info),
            IndexKey::Number if is_applicable_index_type(key, IndexKey::Number) => {
                return Some(info);
            }
            IndexKey::Number => {}
        }
    }
    string_info.filter(|_| is_applicable_index_type(key, IndexKey::String))
}

/// tsc's `getPropertyOfType` on an apparent type, read the way an indexed
/// access type node reads it: an optional property includes `undefined`
/// (`containsMissingType`). An index signature is not a property.
fn property_of_type(apparent: &Type, name: &str) -> Option<Type> {
    match apparent {
        Type::Object(object) => {
            if let Some(property) = object.get_property(name) {
                return Some(if property.is_optional() {
                    union_type(vec![property.ty.clone(), Type::Undefined])
                } else {
                    property.ty.clone()
                });
            }
            if object.string_index_type.is_none() && object.number_index_type.is_none() {
                return apparent.get_property_access_type(name);
            }
            let mut members_only = object.clone();
            members_only.string_index_type = None;
            members_only.number_index_type = None;
            Type::Object(members_only).get_property_access_type(name)
        }
        Type::Tuple(elements) => {
            if name == "length" {
                return Some(Type::NumberLiteral(NumberLiteralType {
                    value: elements.len().to_string(),
                }));
            }
            match tuple_element_position(name) {
                Some(position) => elements.get(position).cloned(),
                None => apparent.get_property_access_type(name),
            }
        }
        Type::OpenTuple(open) => match tuple_element_position(name) {
            Some(position) => open.leading.get(position).cloned(),
            None => apparent.get_property_access_type(name),
        },
        Type::Union(union) => union_property(union.types(), name),
        Type::Any | Type::Never | Type::Void | Type::Undefined | Type::Null => None,
        other if other.is_unknown() => None,
        other => other.get_property_access_type(name),
    }
}

/// tsc's `createUnionOrIntersectionProperty` for a read: a member lacking the
/// property contributes what its index signature reads (a tuple's rest
/// elements, or `undefined` past a fixed tuple's end); a member with neither
/// makes the property partial, which a read does not see.
fn union_property(members: &[Type], name: &str) -> Option<Type> {
    let key = Type::StringLiteral(name.to_string());
    let mut found = false;
    let mut types = Vec::with_capacity(members.len());
    for member in members {
        let apparent = apparent_type(member);
        if matches!(apparent, Type::ErrorType | Type::Never) {
            continue;
        }
        // A member surge could not model contributes itself instead of deciding
        // the read.
        if apparent.is_unknown() || matches!(apparent, Type::Any) {
            found = true;
            types.push(apparent);
            continue;
        }
        if let Some(property) = property_of_type(&apparent, name) {
            found = true;
            types.push(property);
            continue;
        }
        let info = applicable_index_info(index_infos(&apparent), &key)?;
        types.push(if is_tuple_type(&apparent) {
            rest_type_of_tuple(&apparent).unwrap_or(Type::Undefined)
        } else {
            info.value
        });
    }
    found.then(|| union_type(types))
}

/// tsc's `isStringIndexSignatureOnlyType`.
fn is_string_index_signature_only_type(ty: &Type) -> bool {
    match ty {
        Type::Object(object) => {
            object.properties.is_empty()
                && object.string_index_type.is_some()
                && object.number_index_type.is_none()
                && !object.synthetic_open_index
        }
        Type::Union(union) => union
            .types()
            .iter()
            .all(|member| is_string_index_signature_only_type(&apparent_type(member))),
        _ => false,
    }
}

/// How tsc prints the apparent receiver in a missing-member message: a
/// primitive reads as its lib interface.
fn apparent_display_name(apparent: &Type) -> String {
    match apparent {
        Type::String | Type::StringLiteral(_) => "String".to_string(),
        Type::Number | Type::NumberLiteral(_) => "Number".to_string(),
        Type::Boolean | Type::BooleanLiteral(_) => "Boolean".to_string(),
        Type::BigInt => "BigInt".to_string(),
        Type::Symbol => "Symbol".to_string(),
        other => other.name(),
    }
}

/// tsc's `getBindingElementTypeFromParentType`, for what it reports: an object
/// pattern's property the parent does not have (TS2339 at the property name),
/// an array pattern over a type that is not iterable (TS2488 at the pattern),
/// and an element past the end of a fixed tuple without a default (TS2493). A
/// nested pattern reads its element's type.
pub(crate) fn check_binding_pattern_reads(
    binding: &surge_ts_syntax::ParsedBindingName,
    parent: &Type,
    ctx: &mut CheckerContext,
) {
    use surge_ts_syntax::ParsedBindingName;
    let apparent = apparent_type(parent);
    if [parent, &apparent]
        .iter()
        .any(|ty| matches!(ty, Type::Any | Type::Unknown | Type::ErrorType | Type::TypeParameter(_)))
        || receiver_is_unenumerable(&apparent)
    {
        return;
    }
    // A nested pattern over a read that may be missing is reported by
    // `check_binding_pattern_defaults`, which also skips what lies beneath.
    let descend = |binding: &ParsedBindingName, read: Type, has_default: bool, ctx: &mut CheckerContext| {
        let read = if has_default { surge_ts_types::remove_undefined(&read) } else { read };
        if matches!(binding, ParsedBindingName::Identifier { .. }) {
            return;
        }
        if !has_default
            && (surge_ts_types::is_assignable_to(&Type::Undefined, &read)
                || surge_ts_types::is_assignable_to(&Type::Null, &read))
        {
            return;
        }
        check_binding_pattern_reads(binding, &read, ctx);
    };
    match binding {
        // `checkVariableLikeDeclaration` checks an empty object pattern's
        // source with `checkNonNullNonVoidType`.
        ParsedBindingName::ObjectPattern(pattern)
            if pattern.elements.is_empty() && pattern.rest.is_none() =>
        {
            if !surge_ts_types::strict_null_checks() {
                return;
            }
            let null = surge_ts_types::is_assignable_to(&Type::Null, parent);
            // `void` is reported as possibly `undefined`.
            let undefined =
                matches!(parent, Type::Void) || surge_ts_types::is_assignable_to(&Type::Undefined, parent);
            let diagnostic = match (null, undefined) {
                (true, true) => Diagnostic::ts2533(ctx.file_name.clone()),
                (true, false) => Diagnostic::ts2531(ctx.file_name.clone()),
                (false, true) => Diagnostic::ts2532(ctx.file_name.clone()),
                (false, false) => return,
            };
            ctx.push(match pattern.span {
                Some(span) => diagnostic.with_span(convert_span(span)),
                None => diagnostic,
            });
        }
        ParsedBindingName::ObjectPattern(pattern) => {
            for element in &pattern.elements {
                if element.computed_key || matches!(element.binding_name, ParsedBindingName::Unsupported { .. }) {
                    continue;
                }
                let site = AccessSite {
                    span: element.span.map(|span| TextSpan {
                        start: span.start,
                        end: span.start + element.property_name.len(),
                    }),
                };
                let key = Type::StringLiteral(element.property_name.clone());
                let Some(read) = indexed_access_type_or_undefined(&apparent, &key, Some(site), ctx) else {
                    continue;
                };
                descend(&element.binding_name, read, element.has_default, ctx);
            }
        }
        ParsedBindingName::ArrayPattern(pattern) => {
            if crate::checks::expr::is_definitely_not_iterable(&apparent, true) {
                let diagnostic = Diagnostic::ts2488(parent.name(), ctx.file_name.clone());
                ctx.push(match pattern.span {
                    Some(span) => diagnostic.with_span(convert_span(span)),
                    None => diagnostic,
                });
                return;
            }
            for (index, element) in pattern.elements.iter().enumerate() {
                let Some(element) = element else { continue };
                let has_default = pattern.defaults.get(index).copied().unwrap_or(false);
                let read = match &apparent {
                    Type::Tuple(elements) if index >= elements.len() => {
                        if !has_default {
                            let span = match element {
                                ParsedBindingName::Identifier { span, .. }
                                | ParsedBindingName::Unsupported { span } => *span,
                                ParsedBindingName::ObjectPattern(pattern) => pattern.span,
                                ParsedBindingName::ArrayPattern(pattern) => pattern.span,
                            };
                            let key = Type::NumberLiteral(NumberLiteralType { value: index.to_string() });
                            let _ = indexed_access_type_or_undefined(&apparent, &key, Some(AccessSite { span }), ctx);
                        }
                        continue;
                    }
                    Type::Tuple(elements) => elements[index].clone(),
                    Type::Array(element_type) => (**element_type).clone(),
                    _ => continue,
                };
                descend(element, read, has_default, ctx);
            }
        }
        _ => {}
    }
}

/// tsc's `getIndexedAccessTypeOrUndefined` for an access it does not defer.
fn indexed_access_type_or_undefined(
    object: &Type,
    index: &Type,
    site: Option<AccessSite>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let view = index_view(index);
    // A string index signature and nothing else answers every string or numeric
    // key with its value, `{ [k: string]: V }["toString"]` included.
    let index = if is_string_index_signature_only_type(object)
        && !is_nullable(&view)
        && is_assignable_to_key_kind(&view, STRING_OR_NUMBER_KIND)
    {
        Type::String
    } else {
        index.clone()
    };
    let apparent = apparent_type(object);
    // `boolean` alone stays whole, so its error names `boolean`; inside a larger
    // union it is `false | true`.
    let Type::Union(union) = index_view(&index) else {
        return property_type_for_index_type(&apparent, &index, site, ctx);
    };
    let mut members = Vec::with_capacity(union.types().len() + 1);
    for member in union.types() {
        if matches!(member, Type::Boolean) {
            members.push(Type::BooleanLiteral(false));
            members.push(Type::BooleanLiteral(true));
        } else {
            members.push(member.clone());
        }
    }
    let mut property_types = Vec::with_capacity(members.len());
    let mut was_missing = false;
    for member in &members {
        match property_type_for_index_type(&apparent, member, site, ctx) {
            Some(ty) => property_types.push(ty),
            None if site.is_none() => return None,
            None => was_missing = true,
        }
    }
    (!was_missing).then(|| union_type(property_types))
}

/// tsc's `getPropertyTypeForIndexType` for an indexed access type node.
fn property_type_for_index_type(
    apparent: &Type,
    index: &Type,
    site: Option<AccessSite>,
    ctx: &mut CheckerContext,
) -> Option<Type> {
    let view = index_view(index);
    let property_name = match &view {
        Type::StringLiteral(value) => Some(value.as_str()),
        Type::NumberLiteral(number) => Some(number.value.as_str()),
        _ => None,
    };
    if let Some(name) = property_name {
        if let Some(property) = property_of_type(apparent, name) {
            return Some(property);
        }
        if every_member(apparent, is_tuple_type) && is_numeric_literal_name(name) {
            let position = name.parse::<f64>().unwrap_or(f64::NAN);
            if let Some(site) = site
                && every_member(apparent, |member| matches!(member, Type::Tuple(_)))
            {
                if let Type::Tuple(elements) = apparent {
                    if position < 0.0 {
                        site.report(Diagnostic::ts2514(ctx.file_name.clone()), ctx);
                        return Some(Type::Undefined);
                    }
                    let diagnostic = Diagnostic::ts2493(
                        apparent.name(),
                        elements.len(),
                        name,
                        ctx.file_name.clone(),
                    );
                    site.report(diagnostic, ctx);
                } else {
                    let diagnostic =
                        Diagnostic::ts2339(name, apparent.name(), ctx.file_name.clone());
                    site.report(diagnostic, ctx);
                }
            }
            if position >= 0.0 {
                return Some(tuple_element_type_out_of_start_count(apparent));
            }
        }
    }
    // A unique symbol names the member declared under it (`[s]`, or
    // `[ns.s]` through a namespace), tsc's late-bound name.
    if let Type::Reference(reference) = &view
        && let Some(name) = reference.unique_symbol_name()
        && let Type::Object(object) = apparent
    {
        let own = format!("[{name}]");
        let qualified = format!(".{name}]");
        if let Some((_, property)) = object
            .properties
            .iter()
            .find(|(key, _)| key.as_ref() == own.as_str() || key.ends_with(&qualified))
        {
            return Some(property.ty.clone());
        }
    }
    if !is_nullable(&view) && is_assignable_to_key_kind(&view, ANY_KEY_KIND) {
        if matches!(apparent, Type::Any | Type::ErrorType | Type::Never) {
            return Some(apparent.clone());
        }
        // No applicable signature defaults to the string one, so a symbol key
        // reads it too — and is reported below.
        let info = applicable_index_info(index_infos(apparent), &view).or_else(|| {
            index_info_of_type(apparent, IndexKey::String).map(|value| IndexInfo {
                key: IndexKey::String,
                value,
            })
        });
        if let Some(info) = info {
            if let Some(site) = site
                && info.key == IndexKey::String
                && !is_assignable_to_key_kind(&view, STRING_OR_NUMBER_KIND)
            {
                site.report(Diagnostic::ts2538(index.name(), ctx.file_name.clone()), ctx);
            }
            return Some(info.value);
        }
        if matches!(view, Type::Never) {
            return Some(Type::Never);
        }
    }
    if let Some(site) = site {
        let diagnostic = match &view {
            Type::StringLiteral(value) => Some(Diagnostic::ts2339(
                value,
                apparent_display_name(apparent),
                ctx.file_name.clone(),
            )),
            Type::NumberLiteral(number) => Some(Diagnostic::ts2339(
                &number.value,
                apparent_display_name(apparent),
                ctx.file_name.clone(),
            )),
            Type::String | Type::Number => Some(Diagnostic::ts2537(
                apparent_display_name(apparent),
                index.name(),
                ctx.file_name.clone(),
            )),
            // An unresolved index name was already reported (TS2304); the
            // native profile does not cascade a second diagnostic off it.
            Type::ErrorType
                if ctx.options.diagnostic_profile
                    == crate::context::DiagnosticProfile::Native =>
            {
                None
            }
            _ => Some(Diagnostic::ts2538(index.name(), ctx.file_name.clone())),
        };
        if let Some(diagnostic) = diagnostic {
            site.report(diagnostic, ctx);
        }
    }
    matches!(view, Type::Any | Type::ErrorType).then_some(view)
}
