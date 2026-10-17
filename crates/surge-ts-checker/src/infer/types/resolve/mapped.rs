use super::*;

use surge_ts_syntax::{MappedOptionality, ParsedMappedType};
use surge_ts_types::{ObjectProperty, PropertyMap};

use crate::metrics::alloc_object_type;

/// Whether a mapped type's key constraint admits arbitrary members, which makes
/// the result an index signature rather than a fixed property set. Mirrors
/// `record_key_is_open`, the built-in-lib path's rule for the same question.
fn mapped_key_is_open(constraint: &Type) -> bool {
    match constraint {
        Type::String | Type::Number | Type::Symbol => true,
        // `any` is a key tsc accepts too, and it lands on the *string* index:
        // `resolveMappedTypeMembers` takes the `TypeFlagsAny` branch and rewrites
        // `indexKeyType` to `stringType`, which is why `Record<any, any>` is
        // `{ [x: string]: any }`. Without it the mapped type degraded to the
        // sentinel, and `T extends Record<any, any>` then answered backwards.
        Type::Any => true,
        Type::Union(union) => union.types().iter().any(mapped_key_is_open),
        // A pattern key is a string index restricted to the pattern; see
        // `record_key_is_open`.
        other => {
            surge_ts_types::is_template_literal_type(other)
                || surge_ts_types::string_mapping_parts(other).is_some()
        }
    }
}

/// The literal keys a key constraint enumerates, each with the property name it
/// produces. A numeric key names the member by its text, the same way an object
/// literal's numeric key does, while the key parameter stays the number literal.
fn mapped_literal_keys(constraint: &Type) -> Option<Vec<(String, Type)>> {
    match constraint {
        // An empty key set maps to an empty object, not a failure: tsc's
        // `resolveMappedTypeMembers` walks the constraint's constituents and a
        // `never` constraint simply contributes none, leaving the members
        // table empty. Answering `None` here degraded `{ [K in keyof R]: … }`
        // with `R = {}` to the sentinel, and `keyof` of that then decided a
        // conditional from a type surge never resolved.
        Type::Never => Some(Vec::new()),
        Type::StringLiteral(value) => Some(vec![(value.clone(), constraint.clone())]),
        Type::NumberLiteral(literal) => Some(vec![(literal.value.clone(), constraint.clone())]),
        Type::Reference(reference) if reference.is_unique_symbol() => {
            Some(vec![(remapped_property_name(constraint)?, constraint.clone())])
        }
        // An enum is the union of its members, and a member keys the property
        // its value names while the key parameter stays the member.
        Type::Reference(reference) if reference.enum_owner.is_some() => match reference.resolve() {
            Type::StringLiteral(value) => Some(vec![(value, constraint.clone())]),
            Type::NumberLiteral(literal) => Some(vec![(literal.value.clone(), constraint.clone())]),
            resolved @ Type::Union(_) => mapped_literal_keys(&resolved),
            _ => None,
        },
        Type::Union(union) => {
            let mut keys = Vec::new();
            for variant in union.types() {
                keys.extend(mapped_literal_keys(variant)?);
            }
            Some(keys)
        }
        _ => None,
    }
}

/// `resolveMappedTypeMembers`: keys naming the same property (two enums'
/// `"cat"` members) make one property, whose key is their union.
fn merge_same_named_keys(keys: Vec<(String, Type)>) -> Vec<(String, Type)> {
    let mut merged: Vec<(String, Vec<Type>)> = Vec::with_capacity(keys.len());
    let mut positions: surge_ts_types::fx::FxHashMap<String, usize> = surge_ts_types::fx::FxHashMap::default();
    for (name, key_type) in keys {
        match positions.get(&name) {
            Some(&position) => merged[position].1.push(key_type),
            None => {
                positions.insert(name.clone(), merged.len());
                merged.push((name, vec![key_type]));
            }
        }
    }
    merged
        .into_iter()
        .map(|(name, mut types)| {
            let key_type = if types.len() == 1 { types.pop().unwrap_or(Type::Never) } else { surge_ts_types::union_type(types) };
            (name, key_type)
        })
        .collect()
}

/// The property an `as` clause renames a key to: a string literal, or a
/// well-known symbol's `[Symbol.<name>]` member.
fn remapped_property_name(key: &Type) -> Option<String> {
    match key {
        Type::StringLiteral(name) => Some(name.clone()),
        Type::Reference(reference) => reference
            .unique_symbol_name()
            .filter(|name| name.starts_with("Symbol."))
            .map(|name| format!("[{name}]")),
        _ => None,
    }
}

/// `instantiateMappedType`'s `instantiateConstituent` for a homomorphic
/// mapping whose type variable was instantiated to something other than an
/// object or a union: a primitive maps to itself, and without an `as` clause
/// an array or fixed tuple maps element by element (`instantiateMappedArrayType`,
/// `instantiateMappedTupleType`).
fn instantiate_homomorphic_constituent(
    mapped: &ParsedMappedType,
    operand: &ResolvedType,
    array_constrained: bool,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> Option<ResolvedType> {
    if operand.ty.is_unknown() || operand.ty.is_type_variable() {
        return None;
    }
    let (readonly_source, shape) = match &operand.ty {
        Type::Reference(reference) if reference.is_readonly_array() => (true, operand.ty.peeled()),
        _ => (false, operand.ty.clone()),
    };
    let peeled = shape.peeled();
    if matches!(
        peeled,
        Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::Undefined
            | Type::Null
            | Type::Void
            | Type::Never
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_)
    ) {
        return Some(ResolvedType {
            ty: operand.ty.clone(),
            had_error: operand.had_error,
        });
    }
    if mapped.name_type.is_some() {
        return None;
    }
    let optional = mapped_modifier(mapped.optional);
    let readonly = match mapped.readonly {
        MappedOptionality::Keep => readonly_source,
        MappedOptionality::Add => true,
        MappedOptionality::Remove => false,
    };
    let mut had_error = operand.had_error;
    let mut template = |key: Type, is_optional: bool, ctx: &mut CheckerContext, had_error: &mut bool| {
        let mut key_substitution = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
        key_substitution.insert(mapped.key_name.clone(), key);
        // Array and tuple elements are instantiated eagerly (`createTupleType`),
        // not as deferred members, so a template re-entering the mapping here
        // is a cycle.
        let resolved = resolve_parsed_type(*mapped.value_type.clone(), ctx, resolving, &key_substitution);
        *had_error |= resolved.had_error;
        // `instantiateMappedTypeTemplate`.
        if surge_ts_types::strict_null_checks() && optional > 0 && !type_admits_undefined(&resolved.ty) {
            union_type(vec![resolved.ty, Type::Undefined])
        } else if surge_ts_types::strict_null_checks() && optional < 0 && is_optional {
            surge_ts_types::remove_undefined(&resolved.ty)
        } else {
            resolved.ty
        }
    };
    let ty = if let Some((elements, min_length)) = surge_ts_types::fixed_tuple_parts(&shape) {
        let arity = elements.len();
        let mapped_elements = (0..arity)
            .map(|index| template(Type::StringLiteral(index.to_string()), index >= min_length, ctx, &mut had_error))
            .collect::<Vec<_>>();
        let min_length = match optional {
            1 => 0,
            -1 => mapped_elements.len(),
            _ => min_length,
        };
        surge_ts_types::written_labeled_tuple_type(mapped_elements, min_length, surge_ts_types::fixed_tuple_labels(&shape))
    } else if let Type::Array(_) = peeled {
        Type::Array(Box::new(template(Type::Number, true, ctx, &mut had_error)))
    } else if let Type::OpenTuple(open) = &peeled
        && open.trailing.is_empty()
    {
        // `instantiateMappedTupleType` maps a variadic tuple element by element,
        // the rest element through the first index it covers.
        let leading = (0..open.leading.len())
            .map(|index| template(Type::StringLiteral(index.to_string()), optional > 0, ctx, &mut had_error))
            .collect::<Vec<_>>();
        let rest = template(Type::StringLiteral(open.leading.len().to_string()), true, ctx, &mut had_error);
        Type::OpenTuple(surge_ts_types::OpenTupleType {
            leading,
            rest: Box::new(rest),
            trailing: Vec::new(),
            labels: open.labels.clone(),
        })
    } else if matches!(peeled, Type::Any) {
        // `instantiateMappedType` over `any`: an array when the type variable
        // is constrained to arrays and tuples, else the mapping over `any`'s
        // one key, a string index (`forEachMappedTypePropertyKeyTypeAndIndexSignatureKeyType`).
        if array_constrained {
            Type::Array(Box::new(template(Type::Number, true, ctx, &mut had_error)))
        } else {
            let value = template(Type::String, false, ctx, &mut had_error);
            return Some(ResolvedType {
                ty: Type::Object(
                    alloc_object_type(PropertyMap::default(), Some(value))
                        .with_readonly_indexes(readonly, false)
                        .with_open_index_marker(),
                ),
                had_error,
            });
        }
    } else {
        return None;
    };
    Some(ResolvedType {
        ty: if readonly { readonly_reference(ty) } else { ty },
        had_error,
    })
}

thread_local! {
    /// The type parameters of the signature whose annotations are being
    /// instantiated for a call that are constrained to array and tuple types,
    /// which the call's constraint scopes do not carry.
    static ARRAY_CONSTRAINED_SIGNATURE_PARAMETERS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `instantiate` with the array-constrained type parameters among
/// `type_parameters` known to [`constrained_to_arrays`].
pub(crate) fn with_signature_array_constraints<R>(
    type_parameters: &[surge_ts_syntax::ParsedTypeParameter],
    instantiate: impl FnOnce() -> R,
) -> R {
    let names: Vec<String> = type_parameters
        .iter()
        .filter(|parameter| parameter.constraint.as_ref().is_some_and(arrayish_constraint))
        .map(|parameter| parameter.name.clone())
        .collect();
    let saved = ARRAY_CONSTRAINED_SIGNATURE_PARAMETERS.with(|slot| slot.replace(names));
    let result = instantiate();
    ARRAY_CONSTRAINED_SIGNATURE_PARAMETERS.with(|slot| *slot.borrow_mut() = saved);
    result
}

fn arrayish_constraint(ty: &ParsedType) -> bool {
    match ty {
        ParsedType::Array(_) | ParsedType::Tuple(..) | ParsedType::VariadicTuple(..) => true,
        ParsedType::Union(members) => !members.is_empty() && members.iter().all(arrayish_constraint),
        ParsedType::Readonly(inner) => arrayish_constraint(inner),
        ParsedType::Named(named) => matches!(named.name.as_str(), "Array" | "ReadonlyArray"),
        _ => false,
    }
}

/// Whether a type parameter is constrained to array and tuple types only.
fn constrained_to_arrays(name: &str, ctx: &CheckerContext) -> bool {
    match ctx.type_parameter_constraint(name) {
        Some(constraint) => arrayish_constraint(constraint),
        None => ARRAY_CONSTRAINED_SIGNATURE_PARAMETERS.with(|slot| slot.borrow().iter().any(|known| known == name)),
    }
}

fn type_admits_undefined(ty: &Type) -> bool {
    match ty {
        Type::Undefined | Type::Void => true,
        Type::Union(union) => union.types().iter().any(type_admits_undefined),
        _ => false,
    }
}

/// How tsc prints a generic mapped type written without an alias.
fn generic_mapped_display(
    readonly: MappedOptionality,
    optional: MappedOptionality,
    key_name: &str,
    keys: &Type,
    name_type: Option<&Type>,
    template: &Type,
) -> String {
    format!(
        "{{ {}[{} in {}{}]{}: {}; }}",
        match readonly {
            MappedOptionality::Keep => "",
            MappedOptionality::Add => "readonly ",
            MappedOptionality::Remove => "-readonly ",
        },
        key_name,
        keys.name(),
        name_type.map(|name_type| format!(" as {}", name_type.name())).unwrap_or_default(),
        match optional {
            MappedOptionality::Keep => "",
            MappedOptionality::Add => "?",
            MappedOptionality::Remove => "-?",
        },
        template.name(),
    )
}

fn mapped_modifier(modifier: MappedOptionality) -> i8 {
    match modifier {
        MappedOptionality::Keep => 0,
        MappedOptionality::Add => 1,
        MappedOptionality::Remove => -1,
    }
}

/// tsc's `checkMappedType` checks the template whatever the key set resolves
/// to, so a name it cannot find is reported even where the constraint failed
/// and no member is ever instantiated. Only those reports are kept: anything
/// else the template would say is about a key surge never had.
fn report_unresolved_template_names(
    key_name: &str,
    value_type: &ParsedType,
    name_type: Option<&ParsedType>,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) {
    let checkpoint = ctx.diagnostics().len();
    let mut key_substitution = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
    key_substitution.insert(key_name.to_string(), Type::ErrorType);
    for template in std::iter::once(value_type).chain(name_type) {
        let _ = resolve_parsed_type(template.clone(), ctx, resolving, &key_substitution);
    }
    let unresolved_names: Vec<Diagnostic> = ctx.diagnostics()[checkpoint..]
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.code,
                surge_ts_diagnostics::DiagnosticCode::TypeScript(2304 | 2503 | 2552 | 2694 | 2724)
            )
        })
        .cloned()
        .collect();
    ctx.truncate_diagnostics_releasing_utility_keys(checkpoint);
    for diagnostic in unresolved_names {
        ctx.push(diagnostic);
    }
}

fn parsed_type_names_key(ty: &ParsedType, key_name: &str) -> bool {
    let mut names_key = false;
    ty.for_each_named_type(&mut |named| {
        names_key |= named.name == key_name && named.type_arguments.is_empty();
    });
    names_key
}

/// The alias instantiation (`Partial<T>`) a mapped or conditional alias body
/// resolves under: tsc names a deferred one by the alias it is written as.
/// Rendered only when a deferred type takes the name, since naming the
/// arguments of every instantiation is costly.
pub(crate) struct BodyAliasDisplay {
    pub(crate) name: String,
    pub(crate) arguments: Vec<Type>,
}

impl BodyAliasDisplay {
    pub(crate) fn render(&self) -> String {
        let arguments: Vec<String> = self.arguments.iter().map(Type::name).collect();
        format!("{}<{}>", self.name, arguments.join(", "))
    }
}

thread_local! {
    static BODY_ALIAS_DISPLAY: std::cell::RefCell<Option<BodyAliasDisplay>> = const { std::cell::RefCell::new(None) };
}

/// Runs `resolve` with `display` naming the alias body about to resolve. Only
/// the body's own mapped or conditional type reads it.
pub(crate) fn with_body_alias_display<R>(display: Option<BodyAliasDisplay>, resolve: impl FnOnce() -> R) -> R {
    struct Restore(Option<BodyAliasDisplay>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BODY_ALIAS_DISPLAY.with(|pending| *pending.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(BODY_ALIAS_DISPLAY.with(|pending| pending.replace(display)));
    resolve()
}

/// The display [`with_body_alias_display`] published, for the alias body's own
/// type: taken on entry, so a type nested in the body never reads it.
pub(crate) fn take_body_alias_display() -> Option<BodyAliasDisplay> {
    BODY_ALIAS_DISPLAY.with(|pending| pending.borrow_mut().take())
}

pub(crate) fn resolve_mapped_type(
    mapped: ParsedMappedType,
    ctx: &mut CheckerContext,
    resolving: &mut Vec<DeclarationResolutionKey>,
    substitution: &TypeParameterSubstitution,
) -> ResolvedType {
    let body_alias_display = take_body_alias_display();
    let origin_node = crate::checks::function::in_generic_body_read()
        .then(|| surge_ts_syntax::ParsedType::Mapped(std::sync::Arc::new(mapped.clone())));
    // A homomorphic mapping (`[K in keyof X]`) preserves the source's
    // per-property optionality and its string index signature (tsc keeps both;
    // dropping the index signature turned `Flatten<T & Record<string, unknown>>`
    // members into spurious TS2339s). Capture the `keyof` operand so the source
    // shape is recoverable after the constraint is resolved to a key union.
    let keyof_operand: Option<ParsedType> = match mapped.constraint.as_ref() {
        ParsedType::KeyOf(inner) => Some(inner.as_ref().clone()),
        _ => None,
    };
    // `Partial<C1>` with `C1` a type parameter no enclosing declaration binds
    // is an instantiation surge could not complete (the alias argument was
    // lost on the way in); mapping over it would produce an empty object that
    // then reports every member. tsc has the real keys, so the shape is open.
    if let Some(ParsedType::Named(named)) = keyof_operand.as_ref()
        && named.type_arguments.is_empty()
        && let Some(Type::TypeParameter(parameter)) = substitution.get(&named.name)
        && !parameter.is_active_variable()
        && !ctx.type_parameter_in_scope(parameter.name.as_ref())
    {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    }
    // tsc's `instantiateMappedType`: a homomorphic mapping over a type variable
    // instantiated to a union distributes over it, so `Partial<A | B>` is
    // `Partial<A> | Partial<B>` and a member only one side declares survives.
    // A constituent that is not an object (`undefined`, a primitive) maps to
    // itself.
    if let Some(ParsedType::Named(named)) = keyof_operand.as_ref()
        && named.type_arguments.is_empty()
        && substitution.get(&named.name).is_some()
    {
        let operand = resolve_parsed_type(
            ParsedType::Named(named.clone()),
            ctx,
            resolving,
            substitution,
        );
        let array_constrained = constrained_to_arrays(&named.name, ctx);
        if let Some(instantiated) =
            instantiate_homomorphic_constituent(&mapped, &operand, array_constrained, ctx, resolving, substitution)
        {
            return instantiated;
        }
        if let Type::Union(union) = operand.ty.peeled() {
            let mut members = Vec::with_capacity(union.types().len());
            let mut had_error = operand.had_error;
            for member in union.types() {
                if !matches!(member.peeled(), Type::Object(_) | Type::Array(_) | Type::Tuple(_)) {
                    members.push(member.clone());
                    continue;
                }
                let mut member_substitution =
                    substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
                member_substitution.insert(named.name.clone(), member.clone());
                let mapped_member =
                    resolve_mapped_type(mapped.clone(), ctx, resolving, &member_substitution);
                had_error |= mapped_member.had_error;
                members.push(mapped_member.ty);
            }
            return ResolvedType {
                ty: union_type(members),
                had_error,
            };
        }
    }
    // tsc's binder declares the key in the mapped type's own scope, so a
    // constraint naming it reads the key itself, not an outer type of that
    // name: a circular constraint (TS2313, reported on the syntax) or a generic
    // one, neither of which surge enumerates.
    let mut names_own_key = false;
    mapped.constraint.for_each_named_type(&mut |named| {
        names_own_key |= named.name == mapped.key_name && named.type_arguments.is_empty();
    });
    let resolved_constraint = if names_own_key {
        let mut constraint_substitution =
            substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
        constraint_substitution.insert(mapped.key_name.clone(), Type::Unknown);
        resolve_parsed_type(*mapped.constraint, ctx, resolving, &constraint_substitution)
    } else {
        resolve_parsed_type(*mapped.constraint, ctx, resolving, substitution)
    };

    if resolved_constraint.had_error {
        report_unresolved_template_names(
            &mapped.key_name,
            &mapped.value_type,
            mapped.name_type.as_deref(),
            ctx,
            resolving,
            substitution,
        );
        return ResolvedType {
            ty: Type::Unknown,
            had_error: true,
        };
    }
    // `resolveMappedTypeMembers` reads a written `keyof X` off `X` itself
    // (`isMappedTypeWithKeyofConstraintDeclaration`), not off the keys
    // `keyof never` widens to, and `never` has no members. A mapping
    // homomorphic in a type parameter maps `never` to itself above.
    if let Some(operand) = keyof_operand.as_ref()
        && !names_own_key
        && matches!(&resolved_constraint.ty, Type::Union(union)
            if union.types().len() == 3
                && [Type::String, Type::Number, Type::Symbol].iter().all(|key| union.types().contains(key)))
        && matches!(
            resolve_parsed_type(operand.clone(), ctx, resolving, substitution),
            ResolvedType { ty: Type::Never, had_error: false }
        )
    {
        return ResolvedType {
            ty: Type::Object(alloc_object_type(PropertyMap::default(), None)),
            had_error: false,
        };
    }

    // A generic key set (`keyof T`, `K`) leaves the mapped type generic
    // (`isGenericMappedType`): tsc keeps it as a type of its own. surge defers
    // the `{ [P in Q]: X[P] }` shape, `X` a type variable of the body being
    // checked, with the modifiers type `getModifiersTypeFromMappedType` reads.
    if resolved_constraint.ty.is_type_variable()
        && mapped.name_type.is_none()
        && let ParsedType::IndexedAccess(access) = mapped.value_type.as_ref()
        && matches!(access.index_type.as_ref(), ParsedType::Named(index)
            if index.name == mapped.key_name && index.type_arguments.is_empty())
        && !parsed_type_names_key(access.object_type.as_ref(), &mapped.key_name)
    {
        let object = resolve_parsed_type(access.object_type.as_ref().clone(), ctx, resolving, substitution);
        if !object.had_error && object.ty.is_type_variable() {
            let modifiers_type = match keyof_operand.as_ref() {
                Some(operand) => Some(resolve_parsed_type(operand.clone(), ctx, resolving, substitution).ty),
                None => surge_ts_types::type_variable::mapped_modifiers_type(&resolved_constraint.ty),
            };
            let modifiers = surge_ts_types::type_variable::MappedModifiers {
                readonly: mapped_modifier(mapped.readonly),
                optional: mapped_modifier(mapped.optional),
            };
            let name = body_alias_display.as_ref().map(BodyAliasDisplay::render).unwrap_or_else(|| format!(
                "{{ {}[{} in {}]{}: {}[{}]{}; }}",
                match mapped.readonly {
                    MappedOptionality::Keep => "",
                    MappedOptionality::Add => "readonly ",
                    MappedOptionality::Remove => "-readonly ",
                },
                mapped.key_name,
                resolved_constraint.ty.name(),
                match mapped.optional {
                    MappedOptionality::Keep => "",
                    MappedOptionality::Add => "?",
                    MappedOptionality::Remove => "-?",
                },
                object.ty.name(),
                mapped.key_name,
                if matches!(mapped.optional, MappedOptionality::Add) && surge_ts_types::strict_null_checks() {
                    " | undefined"
                } else {
                    ""
                },
            ));
            if let Some(mapped_variable) = surge_ts_types::type_variable::mapped_variable(
                &resolved_constraint.ty,
                &object.ty,
                modifiers,
                modifiers_type.as_ref(),
                name,
            ) {
                if let Some(node) = origin_node.clone() {
                    crate::infer::types::origin::ParsedTypeOrigin::record(&mapped_variable, node, substitution, ctx);
                }
                return ResolvedType {
                    ty: mapped_variable,
                    had_error: false,
                };
            }
        }
    }

    // `{ [P in K]: X }` over a generic key set whose template does not read
    // `P` (`Record<K, T>`) is generic too; its template is the same type for
    // every key.
    if resolved_constraint.ty.is_type_variable()
        && keyof_operand.is_none()
        && mapped.name_type.is_none()
        && !parsed_type_names_key(mapped.value_type.as_ref(), &mapped.key_name)
    {
        let template = resolve_parsed_type(mapped.value_type.as_ref().clone(), ctx, resolving, substitution);
        if !template.had_error && !template.ty.is_unmodelled() {
            let modifiers = surge_ts_types::type_variable::MappedModifiers {
                readonly: mapped_modifier(mapped.readonly),
                optional: mapped_modifier(mapped.optional),
            };
            let name = generic_mapped_display(
                mapped.readonly,
                mapped.optional,
                &mapped.key_name,
                &resolved_constraint.ty,
                None,
                &template.ty,
            );
            if let Some(mapped_variable) = surge_ts_types::type_variable::mapped_constant_variable(
                &resolved_constraint.ty,
                &template.ty,
                modifiers,
                name,
            ) {
                if let Some(node) = origin_node.clone() {
                    crate::infer::types::origin::ParsedTypeOrigin::record(&mapped_variable, node, substitution, ctx);
                }
                return ResolvedType {
                    ty: mapped_variable,
                    had_error: false,
                };
            }
        }
    }

    // Any other mapping over a generic key set is generic in its general
    // shape: the template and `as` clause are read over a key parameter `P`
    // constrained to the keys.
    let generic_key_set = resolved_constraint.ty.is_type_variable()
        || matches!(&resolved_constraint.ty, Type::Object(object)
            if object.properties.is_empty()
                && object.intersection_operands.as_deref().is_some_and(|operands| operands.iter().any(Type::is_type_variable)));
    // A homomorphic mapping over an array or tuple variable maps to an array
    // or tuple once instantiated (`instantiateMappedArrayType`), which the
    // general generic shape does not model.
    let array_like_source = keyof_operand.is_some()
        && surge_ts_types::type_variable::mapped_modifiers_type(&resolved_constraint.ty).is_some_and(|source| {
            surge_ts_types::type_variable::generic_tuple(&source).is_some()
                || matches!(
                    surge_ts_types::type_variable::base_constraint_or_type(&source).peeled(),
                    Type::Array(_) | Type::Tuple(_) | Type::OpenTuple(_)
                )
        });
    if generic_key_set && !array_like_source {
        let declaration: (std::sync::Arc<str>, u32) = (
            std::sync::Arc::from(ctx.file_name.as_str()),
            mapped.key_span.map_or(0, |span| span.start as u32),
        );
        if let Some(key) =
            surge_ts_types::type_variable::mapped_key_variable(&resolved_constraint.ty, declaration, &mapped.key_name)
        {
            let mut key_substitution = substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
            key_substitution.insert(mapped.key_name.clone(), key.clone());
            let name_type = mapped
                .name_type
                .as_deref()
                .map(|name_type| resolve_parsed_type(name_type.clone(), ctx, resolving, &key_substitution));
            let template = resolve_parsed_type(mapped.value_type.as_ref().clone(), ctx, resolving, &key_substitution);
            let modelled = |resolved: &ResolvedType| {
                !resolved.had_error
                    && !(resolved.ty.is_unmodelled() && !matches!(resolved.ty, Type::GenuineUnknown | Type::ErrorType))
            };
            if modelled(&template) && name_type.as_ref().is_none_or(modelled) {
                let modifiers_type = match keyof_operand.as_ref() {
                    Some(operand) => Some(resolve_parsed_type(operand.clone(), ctx, resolving, substitution).ty),
                    None => surge_ts_types::type_variable::mapped_modifiers_type(&resolved_constraint.ty),
                };
                let name = generic_mapped_display(
                    mapped.readonly,
                    mapped.optional,
                    &mapped.key_name,
                    &resolved_constraint.ty,
                    name_type.as_ref().map(|name_type| &name_type.ty),
                    &template.ty,
                );
                if let Some(mapped_variable) = surge_ts_types::type_variable::mapped_generic_variable(
                    surge_ts_types::type_variable::DeferredMapped {
                        key,
                        keys: resolved_constraint.ty.clone(),
                        name_type: name_type.map(|name_type| name_type.ty),
                        template: template.ty,
                        modifiers: surge_ts_types::type_variable::MappedModifiers {
                            readonly: mapped_modifier(mapped.readonly),
                            optional: mapped_modifier(mapped.optional),
                        },
                        modifiers_optionality: modifiers_type
                            .as_ref()
                            .map_or(0, surge_ts_types::type_variable::combined_mapped_optionality),
                    },
                    name,
                ) {
                    if let Some(node) = origin_node.clone() {
                    crate::infer::types::origin::ParsedTypeOrigin::record(&mapped_variable, node, substitution, ctx);
                }
                    return ResolvedType {
                        ty: mapped_variable,
                        had_error: false,
                    };
                }
            }
        }
    }

    // A non-literal key constraint maps to an index signature: `{ [P in string]: T }`
    // is `{ [k: string]: T }`, and `number`/`symbol` are as open as `string`
    // (`keyof any` is all three). This is how `Record<K, T>` resolves when it
    // routes through its mapped-type body — the physical lib declares it as
    // `{ [P in K]: T }` — rather than the built-in `resolve_record_utility_type`
    // fast path. Without this the mapped type collapsed to `unknown`, which
    // surfaced as a spurious missing-property error on every read.
    // `resolveMappedTypeMembers` over a homomorphic mapping of an object:
    // the keys are its properties' names (and its index signatures, handled
    // below), not the reduced `keyof` union an index signature absorbs them
    // into.
    // A constraint naming the mapping's own key (`[P in keyof P]`) is circular
    // (TS2313) and has no source object to read.
    let homomorphic_object = keyof_operand.as_ref().filter(|_| !names_own_key).and_then(|operand| {
        let resolved = resolve_parsed_type(operand.clone(), ctx, resolving, substitution);
        match crate::program::with_dts_expansion_reason(crate::program::DtsExpansionReason::MappedType, || {
            resolved.ty.peeled()
        }) {
            Type::Object(object) if object.string_index_type.is_some() && !object.synthetic_open_index => Some(object),
            _ => None,
        }
    });
    let homomorphic_keys: Option<Vec<(String, Type)>> = homomorphic_object.map(|object| {
        object
            .properties
            .iter()
            .filter(|(key, property)| is_public_key(key, property))
            .filter_map(|(key, _)| Some((key.to_string(), property_name_key_type(key)?)))
            .collect()
    });
    if homomorphic_keys.is_none() && mapped_key_is_open(&resolved_constraint.ty) {
        // The literal-key branch below budgets its expansion; this one resolves
        // the template just the same and needs the same ceiling. It went
        // unguarded only because an open key used to be rare — once `any` keys
        // stopped degrading, drizzle's `Omit`/`Readonly`/intersection chain
        // expanded here without bound and the check never terminated.
        let _expansion_scope = TypeExpansionScope::enter();
        if !try_consume_type_expansion_step() {
            return ResolvedType {
                ty: Type::Unknown,
                had_error: false,
            };
        }
        let mut value_substitution =
            substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
        value_substitution.insert(mapped.key_name.clone(), resolved_constraint.ty.clone());
        // `resolveMappedTypeMembers`: a `readonly` template makes the index
        // signature it maps to read-only.
        let readonly_index = matches!(mapped.readonly, MappedOptionality::Add);
        let resolved_value =
            resolve_parsed_type(*mapped.value_type, ctx, resolving, &value_substitution);
        // A pattern key set is the index signature a written
        // `[k: `a${string}`]` declares, keyed by the patterns.
        let constituents = match &resolved_constraint.ty {
            Type::Union(union) => union.types().to_vec(),
            other => vec![other.clone()],
        };
        let patterns: Option<Vec<Type>> = constituents
            .iter()
            .map(|key| {
                let peeled = surge_ts_types::peel_to_pattern_literal(key);
                (surge_ts_types::is_template_literal_type(&peeled)
                    || surge_ts_types::string_mapping_parts(&peeled).is_some())
                .then_some(peeled)
            })
            .collect();
        let mut object = alloc_object_type(PropertyMap::default(), Some(resolved_value.ty.clone()))
            .with_readonly_indexes(readonly_index, false);
        if let Some(patterns) = patterns
            && !resolved_value.had_error
        {
            object.keyed_indexes = Some(std::sync::Arc::new(surge_ts_types::KeyedIndexes {
                infos: patterns.into_iter().map(|key| (key, resolved_value.ty.clone())).collect(),
                stand_in: Some(resolved_value.ty.clone()),
                symbol: None,
            }));
        }
        return ResolvedType {
            ty: Type::Object(object),
            had_error: resolved_value.had_error,
        };
    }

    let Some(keys) = homomorphic_keys.or_else(|| mapped_literal_keys(&resolved_constraint.ty).map(merge_same_named_keys)) else {
        return ResolvedType {
            ty: Type::Unknown,
            had_error: false,
        };
    };

    // Under `exactOptionalPropertyTypes` the `undefined` a template `X[P]`
    // reads off an optional member is tsc's `missingType`, which an optional
    // mapped member sheds again (`getNonMissingTypeOfSymbol`); surge's
    // optional flag stands for it, so such a member keeps the declared type.
    let indexed_template_object = match mapped.value_type.as_ref() {
        ParsedType::IndexedAccess(access)
            if surge_ts_types::exact_optional_property_types()
                && matches!(access.index_type.as_ref(), ParsedType::Named(index)
                    if index.name == mapped.key_name && index.type_arguments.is_empty()) =>
        {
            Some(access.object_type.as_ref().clone())
        }
        _ => None,
    };
    let template_reads_source = matches!(
        (keyof_operand.as_ref(), indexed_template_object.as_ref()),
        (Some(ParsedType::Named(operand)), Some(ParsedType::Named(object)))
            if operand.type_arguments.is_empty() && object.type_arguments.is_empty() && object.name == operand.name
    );
    let homomorphic_source = keyof_operand.and_then(|operand| {
        let resolved = resolve_parsed_type(operand, ctx, resolving, substitution);
        match crate::program::with_dts_expansion_reason(
            crate::program::DtsExpansionReason::MappedType,
            || resolved.ty.peeled(),
        ) {
            Type::Object(object) => Some(object),
            _ => None,
        }
    });

    let _expansion_scope = TypeExpansionScope::enter();
    let mut properties = PropertyMap::default();
    let mut had_error = false;
    // `resolveMappedTypeMembers`: a key the `as` clause maps to `string` or
    // to the error type is a string index signature valued at the template
    // for that key, unioned across such keys.
    let mut remapped_string_index: Vec<Type> = Vec::new();

    for (key, key_type) in keys {
        if !try_consume_type_expansion_step() {
            return ResolvedType {
                ty: Type::Unknown,
                had_error: false,
            };
        }
        let mut new_substitution =
            substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
        new_substitution.insert(mapped.key_name.clone(), key_type);

        // `as` remaps the key: `never` drops it, a union of literals fans it
        // out, anything else is a shape surge cannot enumerate.
        let mut remapped_to_index = false;
        let property_names: Vec<String> = match mapped.name_type.as_deref() {
            None => vec![key.clone()],
            Some(name_type) => {
                let renamed =
                    resolve_parsed_type(name_type.clone(), ctx, resolving, &new_substitution);
                had_error |= renamed.had_error;
                match renamed.ty {
                    Type::Never => continue,
                    Type::String | Type::ErrorType if !renamed.had_error => {
                        remapped_to_index = true;
                        Vec::new()
                    }
                    Type::Union(union) => {
                        let names: Option<Vec<String>> =
                            union.types().iter().map(remapped_property_name).collect();
                        match names {
                            Some(names) => names,
                            None => {
                                had_error = true;
                                continue;
                            }
                        }
                    }
                    other => match remapped_property_name(&other) {
                        Some(name) => vec![name],
                        None => {
                            had_error = true;
                            continue;
                        }
                    },
                }
            }
        };

        // A mapped property's template is a member like a type literal's, and
        // tsc instantiates it on demand (`getTemplateTypeFromMappedType`), so a
        // generic alias re-entering itself through it — tRPC's
        // `DecoratedProcedureRecord<TRoot, $Value>` for a nested router — is
        // legal recursion that peels lazily, not a cycle to degrade.
        let member_frame = resolving.len();
        ctx.structural_resolution_frames.push(member_frame);
        ctx.type_literal_member_frames.push(member_frame);
        let resolved_value = resolve_parsed_type(
            *mapped.value_type.clone(),
            ctx,
            resolving,
            &new_substitution,
        );
        ctx.type_literal_member_frames.pop();
        ctx.structural_resolution_frames.pop();

        if resolved_value.had_error {
            had_error = true;
        }
        if remapped_to_index {
            remapped_string_index.push(resolved_value.ty);
            continue;
        }

        let source_property = homomorphic_source
            .as_ref()
            .and_then(|object| object.get_property(&key));
        let source_optional = source_property.is_some_and(|property| property.is_optional());
        let source_method = source_property.is_some_and(|property| property.is_method());
        let readonly = match mapped.readonly {
            MappedOptionality::Keep => source_property.is_some_and(|property| property.readonly),
            MappedOptionality::Add => true,
            MappedOptionality::Remove => false,
        };
        // `-?` strips `undefined` from the mapped property as well as clearing
        // the optional flag; that is what makes `Required<{ b?: number }>` a
        // `number` rather than a required `number | undefined`.
        let optional_member = match mapped.optional {
            MappedOptionality::Keep => source_optional,
            MappedOptionality::Add => true,
            MappedOptionality::Remove => false,
        };
        let read_member = match &indexed_template_object {
            Some(_) if !optional_member || resolved_value.had_error => None,
            Some(_) if template_reads_source => source_property.cloned(),
            Some(object) => {
                let resolved = resolve_parsed_type(object.clone(), ctx, resolving, &new_substitution);
                match resolved.ty.peeled() {
                    Type::Object(object) if !resolved.had_error => object.get_property(&key).cloned(),
                    _ => None,
                }
            }
            None => None,
        };
        let template_type = match read_member {
            Some(member) if member.is_optional() => member.ty,
            _ => resolved_value.ty.clone(),
        };
        let (property_type, optional) = match mapped.optional {
            MappedOptionality::Keep => (template_type, source_optional),
            MappedOptionality::Add => (template_type, true),
            MappedOptionality::Remove => {
                (surge_ts_types::remove_undefined(&resolved_value.ty), false)
            }
        };
        for name in property_names {
            properties.insert(
                name.into(),
                ObjectProperty {
                    ty: property_type.clone(),
                    optional,
                    method: source_method,
                    readonly,
                    restriction: None,
                    index_slot: false,
                    written_name: None,
                },
            );
        }
    }

    // `resolveMappedTypeMembers`: each index signature of a homomorphic
    // mapping's source becomes one of its own, valued at the template read with
    // the signature's key type. The template is instantiated with no access
    // node, so whatever the read would report is not reported. An `as` clause
    // and a source the checker left open keep the source's value.
    let index_type = match homomorphic_source.as_ref() {
        Some(source)
            if source.string_index_type.is_some()
                && !source.synthetic_open_index
                && mapped.name_type.is_none() =>
        {
            if !try_consume_type_expansion_step() {
                return ResolvedType {
                    ty: Type::Unknown,
                    had_error: false,
                };
            }
            let mut index_substitution =
                substitution.clone_with_reason(TypeCopyReason::SubstitutionChanged);
            index_substitution.insert(mapped.key_name.clone(), Type::String);
            let diagnostics_before = ctx.diagnostics().len();
            let member_frame = resolving.len();
            ctx.structural_resolution_frames.push(member_frame);
            ctx.type_literal_member_frames.push(member_frame);
            let resolved_value = resolve_parsed_type(
                *mapped.value_type.clone(),
                ctx,
                resolving,
                &index_substitution,
            );
            ctx.type_literal_member_frames.pop();
            ctx.structural_resolution_frames.pop();
            ctx.truncate_diagnostics_releasing_utility_keys(diagnostics_before);
            Some(if resolved_value.had_error {
                Type::Unknown
            } else {
                resolved_value.ty
            })
        }
        Some(source) => source.string_index_type.as_deref().cloned(),
        None => None,
    };
    // A source the checker had to leave open (a spread of a value it could
    // not model) stays open through the mapping: `Partial<{ x, ...degraded }>`
    // still admits the members tsc sees through the spread.
    let source_is_open = homomorphic_source
        .as_ref()
        .is_some_and(|object| object.synthetic_open_index);
    // `resolveMappedTypeMembers`: the index signature is read-only under a
    // `readonly` template, or when the source's is and the template keeps it.
    let index_readonly = match mapped.readonly {
        MappedOptionality::Keep => homomorphic_source
            .as_ref()
            .is_some_and(|object| object.string_index_readonly),
        MappedOptionality::Add => true,
        MappedOptionality::Remove => false,
    };
    let index_type = match (index_type, remapped_string_index.is_empty()) {
        (index_type, true) => index_type,
        (None, false) => Some(union_type(remapped_string_index)),
        (Some(index_type), false) => {
            remapped_string_index.push(index_type);
            Some(union_type(remapped_string_index))
        }
    };
    let mut mapped_object =
        alloc_object_type(properties, index_type).with_readonly_indexes(index_readonly, false);
    if source_is_open {
        mapped_object = mapped_object.with_open_index_marker();
    }

    ResolvedType {
        ty: Type::Object(mapped_object),
        had_error,
    }
}
