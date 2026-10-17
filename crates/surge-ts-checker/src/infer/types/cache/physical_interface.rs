use std::sync::Arc;

use surge_ts_types::{FunctionType, Type};

use super::canonical_declaration_file_name;
use crate::context::{
    CanonicalTypeIdentity, CheckerContext, InterfaceDeclarationTemplate,
    InterfaceEnvironmentIdentity, InterfaceInstantiationKey, InterfaceMemberDeclarationKind,
    InterfaceMemberDeclarationTemplate, InterfaceMemberInstantiationKey,
    InterfaceMethodOverloadGroupTemplate, InterfaceOverloadInstantiationKey,
    StableInterfaceDeclarationFragmentId, StableInterfaceDeclarationId,
    StableInterfaceMemberDeclarationId,
};
use crate::infer::types::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InterfaceCacheSkipReason {
    Disabled,
    UnstableDeclaration,
    UnresolvedTypeArgument,
    UnsupportedTypeArgument,
}

pub(crate) fn physical_interface_cache_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SURGE_DISABLE_PHYSICAL_INTERFACE_CACHE").as_deref() != Ok("1")
    })
}

pub(crate) fn physical_interface_member_cache_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SURGE_DISABLE_PHYSICAL_INTERFACE_MEMBER_CACHE").as_deref() != Ok("1")
    })
}

pub(crate) fn physical_interface_declaration_template(
    ctx: &CheckerContext,
    interface: &crate::symbols::InterfaceInfo,
    declaration: &StableInterfaceDeclarationId,
) -> Option<Arc<InterfaceDeclarationTemplate>> {
    let session = crate::speculative::active_check_session()
        .filter(|session| session.owns_templates(&ctx.physical_interface_declaration_templates));
    let cached = if let Some(session) = session.as_ref() {
        session.template_lookup(declaration)
    } else {
        ctx.physical_interface_declaration_templates
            .lock()
            .ok()
            .and_then(|cache| cache.get(declaration).cloned())
    };
    if let Some(template) = cached {
        crate::program::record_program_counter(|c| {
            c.interface_template_hit_count += 1;
        });
        return Some(template);
    }

    crate::program::record_program_counter(|c| c.interface_template_build_attempt_count += 1);
    if interface.body.members.len() != interface.body.member_fragments.len() {
        return None;
    }

    let mut overload_indices = surge_ts_types::fx::FxHashMap::<&str, u32>::default();
    let mut group_indices = surge_ts_types::fx::FxHashMap::<&str, u32>::default();
    let mut group_members = Vec::<Vec<StableInterfaceMemberDeclarationId>>::new();
    let mut members = Vec::with_capacity(interface.body.members.len());
    for (member, fragment) in interface
        .body
        .members
        .iter()
        .zip(interface.body.member_fragments.iter())
    {
        let declaration_kind = if matches!(member.ty, ParsedType::Function(_)) {
            InterfaceMemberDeclarationKind::Method
        } else {
            InterfaceMemberDeclarationKind::Property
        };
        let overload_index = if declaration_kind == InterfaceMemberDeclarationKind::Method {
            let next = overload_indices.entry(member.name.as_str()).or_default();
            let current = *next;
            *next = next.checked_add(1)?;
            current
        } else {
            0
        };
        let member_declaration = StableInterfaceMemberDeclarationId {
            containing_interface: declaration.clone(),
            canonical_file: canonical_declaration_file_name(&fragment.file_name),
            declaration_start: u32::try_from(member.name_span?.start).ok()?,
            declaration_kind,
            declared_name: Arc::from(member.name.as_str()),
            overload_index,
        };
        let (overload_group, overload_position) =
            if declaration_kind == InterfaceMemberDeclarationKind::Method {
                let group = match group_indices.get(member.name.as_str()).copied() {
                    Some(group) => group,
                    None => {
                        let group = u32::try_from(group_members.len()).ok()?;
                        group_indices.insert(member.name.as_str(), group);
                        group_members.push(Vec::new());
                        group
                    }
                };
                let group_members = group_members.get_mut(group as usize)?;
                let position = u32::try_from(group_members.len()).ok()?;
                group_members.push(member_declaration.clone());
                (Some(group), position)
            } else {
                (None, 0)
            };
        members.push(InterfaceMemberDeclarationTemplate {
            declaration: member_declaration,
            overload_group,
            overload_position,
        });
    }

    let method_groups = group_members
        .into_iter()
        .map(|ordered_members| InterfaceMethodOverloadGroupTemplate {
            ordered_members: Arc::from(ordered_members),
        })
        .collect::<Vec<_>>();
    let retained_bytes = std::mem::size_of::<InterfaceDeclarationTemplate>()
        .saturating_add(members.len() * std::mem::size_of::<InterfaceMemberDeclarationTemplate>())
        .saturating_add(
            method_groups
                .iter()
                .map(|group| {
                    std::mem::size_of::<InterfaceMethodOverloadGroupTemplate>()
                        + group.ordered_members.len()
                            * std::mem::size_of::<StableInterfaceMemberDeclarationId>()
                })
                .sum::<usize>(),
        ) as u64;
    let template = Arc::new(InterfaceDeclarationTemplate {
        members: Arc::from(members),
        method_groups: Arc::from(method_groups),
    });
    if let Some(session) = session {
        return Some(session.template_intern(declaration.clone(), template, retained_bytes));
    }
    let Ok(mut cache) = ctx.physical_interface_declaration_templates.lock() else {
        return Some(template);
    };
    let template = cache
        .entry(declaration.clone())
        .or_insert_with(|| {
            crate::program::record_program_counter(|c| {
                c.interface_template_insert_count += 1;
                c.interface_template_retained_bytes += retained_bytes;
            });
            template
        })
        .clone();
    Some(template)
}

pub(crate) fn interface_member_instantiation_key(
    member: &StableInterfaceMemberDeclarationId,
    interface: &InterfaceInstantiationKey,
) -> InterfaceMemberInstantiationKey {
    InterfaceMemberInstantiationKey {
        member: member.clone(),
        substitution: interface.substitution,
        environment: interface.environment,
    }
}

pub(crate) fn lookup_physical_interface_method(
    ctx: &CheckerContext,
    key: &InterfaceMemberInstantiationKey,
) -> Option<FunctionType> {
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_methods(&ctx.physical_interface_method_instantiations))
    {
        return session.method_lookup(key);
    }
    ctx.physical_interface_method_instantiations
        .lock()
        .ok()?
        .get(key)
        .cloned()
}

pub(crate) fn intern_physical_interface_method(
    ctx: &CheckerContext,
    key: InterfaceMemberInstantiationKey,
    function: FunctionType,
) -> FunctionType {
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_methods(&ctx.physical_interface_method_instantiations))
    {
        let key_bytes = std::mem::size_of::<InterfaceMemberInstantiationKey>() as u64;
        let value_bytes = interface_function_value_shallow_bytes(&function) as u64;
        return session.method_intern(key, function, key_bytes, value_bytes);
    }
    let Ok(mut cache) = ctx.physical_interface_method_instantiations.lock() else {
        return function;
    };
    if let Some(existing) = cache.get(&key) {
        return existing.clone();
    }
    let key_bytes = std::mem::size_of::<InterfaceMemberInstantiationKey>();
    let value_bytes = interface_function_value_shallow_bytes(&function);
    cache.insert(key, function.clone());
    crate::program::record_program_counter(|c| {
        c.interface_method_cache_insert_count += 1;
        c.interface_method_cache_key_bytes += key_bytes as u64;
        c.interface_method_cache_value_shallow_bytes += value_bytes as u64;
    });
    function
}

pub(crate) fn interface_overload_instantiation_key(
    declaration: &StableInterfaceDeclarationId,
    group: &InterfaceMethodOverloadGroupTemplate,
    prefix_len: u32,
    interface: &InterfaceInstantiationKey,
) -> InterfaceOverloadInstantiationKey {
    InterfaceOverloadInstantiationKey {
        containing_interface: declaration.clone(),
        ordered_members: group.ordered_members.clone(),
        prefix_len,
        substitution: interface.substitution,
        environment: interface.environment,
    }
}

pub(crate) fn lookup_physical_interface_overload(
    ctx: &CheckerContext,
    key: &InterfaceOverloadInstantiationKey,
) -> Option<FunctionType> {
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_overloads(&ctx.physical_interface_overload_instantiations))
    {
        return session.overload_lookup(key);
    }
    ctx.physical_interface_overload_instantiations
        .lock()
        .ok()?
        .get(key)
        .cloned()
}

pub(crate) fn intern_physical_interface_overload(
    ctx: &CheckerContext,
    key: InterfaceOverloadInstantiationKey,
    function: FunctionType,
) -> FunctionType {
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_overloads(&ctx.physical_interface_overload_instantiations))
    {
        let key_bytes = std::mem::size_of::<InterfaceOverloadInstantiationKey>() as u64;
        let value_bytes = interface_function_value_shallow_bytes(&function) as u64;
        return session.overload_intern(key, function, key_bytes, value_bytes);
    }
    let Ok(mut cache) = ctx.physical_interface_overload_instantiations.lock() else {
        return function;
    };
    if let Some(existing) = cache.get(&key) {
        return existing.clone();
    }
    let key_bytes = std::mem::size_of::<InterfaceOverloadInstantiationKey>();
    let value_bytes = interface_function_value_shallow_bytes(&function);
    cache.insert(key, function.clone());
    crate::program::record_program_counter(|c| {
        c.interface_overload_cache_insert_count += 1;
        c.interface_overload_cache_key_bytes += key_bytes as u64;
        c.interface_overload_cache_value_shallow_bytes += value_bytes as u64;
    });
    function
}

pub(super) fn interface_function_value_shallow_bytes(function: &FunctionType) -> usize {
    std::mem::size_of::<FunctionType>()
        + std::mem::size_of::<surge_ts_types::FunctionTypePayload>()
        + function.parameters().len() * std::mem::size_of::<Type>()
}

#[cfg(test)]
fn canonical_physical_interface_key(
    interface: &crate::symbols::InterfaceInfo,
    substitution: &TypeParameterSubstitution,
    ctx: &CheckerContext,
    widened: bool,
) -> Result<InterfaceInstantiationKey, InterfaceCacheSkipReason> {
    let declaration = stable_interface_declaration_id(interface)?;
    canonical_physical_interface_key_with_declaration(
        interface,
        substitution,
        ctx,
        declaration,
        widened,
    )
}

pub(crate) fn stable_interface_declaration_id(
    interface: &crate::symbols::InterfaceInfo,
) -> Result<StableInterfaceDeclarationId, InterfaceCacheSkipReason> {
    let memoized = interface
        .cached_stable_id
        .get_or_init(|| build_stable_interface_declaration_id(interface).ok())
        .clone();
    debug_assert_eq!(
        memoized,
        build_stable_interface_declaration_id(interface).ok(),
        "stale cached_stable_id: a fragment or rename mutation missed its reset"
    );
    memoized.ok_or(InterfaceCacheSkipReason::UnstableDeclaration)
}

pub(super) fn build_stable_interface_declaration_id(
    interface: &crate::symbols::InterfaceInfo,
) -> Result<StableInterfaceDeclarationId, InterfaceCacheSkipReason> {
    let declaration_start = interface
        .name_span
        .map_or(Ok(0), |span| u32::try_from(span.start))
        .map_err(|_| InterfaceCacheSkipReason::UnstableDeclaration)?;
    let mut merged_fragments = Vec::with_capacity(interface.body.declaration_fragments.len());
    for fragment in &interface.body.declaration_fragments {
        merged_fragments.push(StableInterfaceDeclarationFragmentId {
            canonical_file: canonical_declaration_file_name(&fragment.file_name),
            declaration_start: u32::try_from(fragment.declaration_start)
                .map_err(|_| InterfaceCacheSkipReason::UnstableDeclaration)?,
        });
    }
    Ok(StableInterfaceDeclarationId {
        canonical_file: canonical_declaration_file_name(&interface.file_name),
        declaration_start,
        declaration_name: Arc::from(
            interface
                .declared_name
                .as_deref()
                .unwrap_or(&interface.name),
        ),
        merged_fragments: Arc::from(merged_fragments),
    })
}

/// EXPERIMENT KNOB (`SURGE_IFACE_KEY_ENV=0`): drop the environment component of
/// `InterfaceInstantiationKey`, restoring the pre-2026-09-11 key. Measurement
/// only — the weak key shares a user interface's expansion across resolution
/// environments, which the memory-lifetime rules forbid. It exists to size how
/// much of the cache's miss rate the environment component is responsible for.
pub(super) fn interface_key_environment_discriminator(ctx: &CheckerContext) -> u64 {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var("SURGE_IFACE_KEY_ENV").as_deref() != Ok("0")) {
        return 0;
    }
    ctx.declaration_environment()
        .canonicalization_discriminator()
}

pub(crate) fn canonical_physical_interface_key_with_declaration(
    interface: &crate::symbols::InterfaceInfo,
    substitution: &TypeParameterSubstitution,
    ctx: &CheckerContext,
    declaration: StableInterfaceDeclarationId,
    widened: bool,
) -> Result<InterfaceInstantiationKey, InterfaceCacheSkipReason> {
    let mut arguments = Vec::with_capacity(interface.body.type_parameters.len());
    let mut budget = 128usize;
    for parameter in &interface.body.type_parameters {
        let Some(argument) = substitution.get(&parameter.name) else {
            return Err(InterfaceCacheSkipReason::UnresolvedTypeArgument);
        };
        // A placeholder slot whose value is a `Type::TypeParameter` is the
        // declaration's own parameter standing for itself, which is a perfectly
        // good cache key. A placeholder slot holding anything else was filled by
        // a resolution that may have degraded to `Unknown`, and the
        // memory-lifetime rules forbid caching that — keep refusing it.
        if substitution.is_placeholder(&parameter.name)
            && !matches!(argument, Type::TypeParameter(_))
        {
            return Err(InterfaceCacheSkipReason::UnresolvedTypeArgument);
        }
        // Display-inclusive identity: the deep display fingerprint keeps
        // structurally-equal-but-differently-rendered arguments apart, so a
        // cached instantiation never substitutes another context's rendering
        // (the canonical-store display-substitution class).
        arguments.push(CanonicalTypeIdentity::DisplayTagged(
            Box::new(canonical_type_identity(argument, 0, &mut budget, widened)?),
            crate::speculative::display_type_fingerprint(argument),
        ));
    }

    let substitution = ctx
        .substitution_store
        .intern(declaration.clone(), arguments);
    Ok(InterfaceInstantiationKey {
        declaration,
        substitution,
        environment: InterfaceEnvironmentIdentity {
            no_lib: ctx.options.no_lib,
            skip_lib_check: ctx.options.skip_lib_check,
            environment_discriminator: interface_key_environment_discriminator(ctx),
        },
    })
}

pub(super) fn canonical_type_identity(
    ty: &Type,
    depth: usize,
    budget: &mut usize,
    widened: bool,
) -> Result<CanonicalTypeIdentity, InterfaceCacheSkipReason> {
    if depth >= 32 || *budget == 0 {
        return Err(InterfaceCacheSkipReason::UnsupportedTypeArgument);
    }
    *budget -= 1;

    let primitive = match ty {
        Type::String => Some(CanonicalTypeIdentity::String),
        Type::Number => Some(CanonicalTypeIdentity::Number),
        Type::Boolean => Some(CanonicalTypeIdentity::Boolean),
        Type::BigInt => Some(CanonicalTypeIdentity::BigInt),
        Type::Symbol => Some(CanonicalTypeIdentity::Symbol),
        Type::Undefined => Some(CanonicalTypeIdentity::Undefined),
        Type::Null => Some(CanonicalTypeIdentity::Null),
        Type::Void => Some(CanonicalTypeIdentity::Void),
        Type::Any => Some(CanonicalTypeIdentity::Any),
        Type::Never => Some(CanonicalTypeIdentity::Never),
        Type::StringLiteral(value) => Some(CanonicalTypeIdentity::StringLiteral(Arc::from(
            value.as_str(),
        ))),
        Type::NumberLiteral(value) => Some(CanonicalTypeIdentity::NumberLiteral(Arc::from(
            value.value.as_str(),
        ))),
        Type::BooleanLiteral(value) => Some(CanonicalTypeIdentity::BooleanLiteral(*value)),
        Type::TypeParameter(parameter) => {
            Some(CanonicalTypeIdentity::TypeParameter(parameter.name.clone()))
        }
        _ => None,
    };
    if let Some(identity) = primitive {
        return Ok(identity);
    }

    match ty {
        Type::Array(element) => Ok(CanonicalTypeIdentity::Array(Box::new(
            canonical_type_identity(element, depth + 1, budget, widened)?,
        ))),
        Type::Tuple(elements) => {
            let mut identities = Vec::with_capacity(elements.len());
            for element in elements {
                identities.push(canonical_type_identity(
                    element,
                    depth + 1,
                    budget,
                    widened,
                )?);
            }
            Ok(CanonicalTypeIdentity::Tuple(Arc::from(identities)))
        }
        Type::Reference(reference) => {
            let mut arguments = Vec::with_capacity(reference.arguments.len());
            for argument in reference.arguments.iter() {
                arguments.push(canonical_type_identity(
                    argument,
                    depth + 1,
                    budget,
                    widened,
                )?);
            }
            Ok(CanonicalTypeIdentity::Reference {
                declaration: reference.id.clone(),
                arguments: Arc::from(arguments),
            })
        }
        Type::Object(object) if widened => {
            // Structural, equality-faithful encoding: exactly the fields
            // `ObjectType`'s equality compares (`properties` +
            // `string_index_type`), property order canonicalized by name to
            // match the order-independent `IndexMap` equality. The lib tier's
            // alias fast-path is NOT reused here: `alias_id` does not
            // participate in object equality.
            let mut properties = Vec::with_capacity(object.properties.len());
            for (name, property) in object.properties.iter() {
                properties.push((
                    name.clone(),
                    property.optional,
                    canonical_type_identity(&property.ty, depth + 1, budget, widened)?,
                ));
            }
            properties.sort_by(|a, b| a.0.cmp(&b.0));
            let string_index = match &object.string_index_type {
                Some(index) => Some(Box::new(canonical_type_identity(
                    index,
                    depth + 1,
                    budget,
                    widened,
                )?)),
                None => None,
            };
            Ok(CanonicalTypeIdentity::ObjectArg {
                properties: Arc::from(properties),
                string_index,
            })
        }
        Type::Object(object) => object
            .alias_id
            .clone()
            .map(CanonicalTypeIdentity::NamedObject)
            .ok_or(InterfaceCacheSkipReason::UnsupportedTypeArgument),
        Type::Unknown | Type::GenuineUnknown | Type::ErrorType | Type::TypeParameter(_) => {
            Err(InterfaceCacheSkipReason::UnresolvedTypeArgument)
        }
        Type::Union(union) if widened => {
            let mut members = Vec::with_capacity(union.types().len());
            for member in union.types() {
                members.push(canonical_type_identity(member, depth + 1, budget, widened)?);
            }
            Ok(CanonicalTypeIdentity::UnionArg {
                list_id: union.list_id(),
                members: Arc::from(members),
            })
        }
        Type::Function(function) if widened => {
            let mut parameters = Vec::with_capacity(function.parameters().len());
            for parameter in function.parameters() {
                parameters.push(canonical_type_identity(
                    parameter,
                    depth + 1,
                    budget,
                    widened,
                )?);
            }
            Ok(CanonicalTypeIdentity::FunctionArg {
                parameter_list_id: function.parameter_list_id(),
                parameters: Arc::from(parameters),
                return_type: Box::new(canonical_type_identity(
                    function.return_type(),
                    depth + 1,
                    budget,
                    widened,
                )?),
                is_variadic: function.is_variadic(),
                required_parameter_count: function.required_parameter_count(),
            })
        }
        Type::Function(_) | Type::Union(_) => {
            Err(InterfaceCacheSkipReason::UnsupportedTypeArgument)
        }
        // Anything without a canonical identity is simply not cached.
        _ => Err(InterfaceCacheSkipReason::UnsupportedTypeArgument),
    }
}

pub(crate) fn lookup_physical_interface_instantiation(
    ctx: &CheckerContext,
    key: &InterfaceInstantiationKey,
) -> Option<Arc<Type>> {
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_physical(&ctx.physical_interface_instantiations))
    {
        return session.physical_lookup(key);
    }
    let cache = ctx.physical_interface_instantiations.lock().ok()?;
    cache.get(key).cloned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InterfaceCacheValueRejection {
    Unknown,
    ResolutionContext,
    TraversalLimit,
}

pub(crate) fn validate_physical_interface_cache_value(
    resolved: &Type,
) -> Result<(), InterfaceCacheValueRejection> {
    fn visit(
        ty: &Type,
        depth: usize,
        budget: &mut usize,
    ) -> Result<(), InterfaceCacheValueRejection> {
        if depth >= 64 || *budget == 0 {
            return Err(InterfaceCacheValueRejection::TraversalLimit);
        }
        *budget -= 1;
        match ty {
            Type::Function(function) => {
                for parameter in function.parameters() {
                    visit(parameter, depth + 1, budget)?;
                }
                visit(function.return_type(), depth + 1, budget)
            }
            Type::Object(object) => {
                for property in object.properties.values() {
                    visit(&property.ty, depth + 1, budget)?;
                }
                if let Some(indexed) = object.string_index_type.as_deref() {
                    visit(indexed, depth + 1, budget)?;
                }
                for signature in [object.construct_signature(), object.call_signature()]
                    .into_iter()
                    .flatten()
                {
                    for parameter in signature.parameters() {
                        visit(parameter, depth + 1, budget)?;
                    }
                    visit(signature.return_type(), depth + 1, budget)?;
                }
                Ok(())
            }
            Type::Array(element) => visit(element, depth + 1, budget),
            Type::Tuple(elements) => {
                for element in elements {
                    visit(element, depth + 1, budget)?;
                }
                Ok(())
            }
            Type::Union(union) => {
                for member in union.types() {
                    visit(member, depth + 1, budget)?;
                }
                Ok(())
            }
            Type::Reference(reference) => {
                if reference.retains_resolution_context() {
                    return Err(InterfaceCacheValueRejection::ResolutionContext);
                }
                for argument in reference.arguments.iter() {
                    visit(argument, depth + 1, budget)?;
                }
                Ok(())
            }
            Type::Unknown | Type::TypeParameter(_) => Err(InterfaceCacheValueRejection::Unknown),
            _ => Ok(()),
        }
    }

    visit(resolved, 0, &mut 512)
}

pub(crate) fn physical_interface_method_has_contextual_typing_dependency(resolved: &Type) -> bool {
    fn contains_callable(ty: &Type, depth: usize, budget: &mut usize) -> bool {
        if depth >= 32 || *budget == 0 {
            return true;
        }
        *budget -= 1;
        match ty {
            Type::Function(_) => true,
            Type::Object(object) => {
                object.call_signature().is_some()
                    || object.construct_signature().is_some()
                    || object
                        .properties
                        .values()
                        .any(|property| contains_callable(&property.ty, depth + 1, budget))
            }
            Type::Array(element) => contains_callable(element, depth + 1, budget),
            Type::Tuple(elements) => elements
                .iter()
                .any(|element| contains_callable(element, depth + 1, budget)),
            Type::Union(union) => union
                .types()
                .iter()
                .any(|member| contains_callable(member, depth + 1, budget)),
            _ => false,
        }
    }

    let Type::Function(function) = resolved else {
        return false;
    };
    let mut budget = 128;
    function
        .parameters()
        .iter()
        .any(|parameter| contains_callable(parameter, 0, &mut budget))
}

pub(crate) fn intern_physical_interface_instantiation(
    ctx: &CheckerContext,
    key: InterfaceInstantiationKey,
    resolved: Type,
) -> Arc<Type> {
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_physical(&ctx.physical_interface_instantiations))
    {
        return session.physical_intern(key, resolved);
    }
    let Ok(mut cache) = ctx.physical_interface_instantiations.lock() else {
        return Arc::new(resolved);
    };
    if let Some(existing) = cache.get(&key) {
        crate::program::record_program_counter(|c| {
            c.physical_interface_cache_racing_insert_count += 1
        });
        return existing.clone();
    }

    let key_bytes = interface_key_shallow_bytes(&key);
    let value_bytes = interface_value_shallow_bytes(&resolved);
    let resolved = Arc::new(resolved);
    cache.insert(key, resolved.clone());
    crate::program::record_program_counter(|c| {
        c.physical_interface_cache_insert_count += 1;
        c.physical_interface_cache_key_bytes += key_bytes;
        c.physical_interface_cache_value_shallow_bytes += value_bytes;
    });
    resolved
}

pub(super) fn interface_key_shallow_bytes(key: &InterfaceInstantiationKey) -> u64 {
    let mut bytes = std::mem::size_of::<InterfaceInstantiationKey>()
        + key.declaration.canonical_file.len()
        + key.declaration.declaration_name.len();
    bytes += key
        .declaration
        .merged_fragments
        .iter()
        .map(|fragment| {
            std::mem::size_of::<StableInterfaceDeclarationFragmentId>()
                + fragment.canonical_file.len()
        })
        .sum::<usize>();
    bytes as u64
}

pub(super) fn interface_value_shallow_bytes(resolved: &Type) -> u64 {
    let mut bytes = std::mem::size_of::<Type>();
    if let Type::Object(object) = resolved {
        bytes += std::mem::size_of::<surge_ts_types::ObjectType>();
        bytes += object.properties.capacity()
            * (std::mem::size_of::<String>()
                + std::mem::size_of::<surge_ts_types::ObjectProperty>());
    }
    bytes as u64
}

#[cfg(test)]
mod physical_interface_cache_tests {
    use std::sync::Arc;

    use surge_ts_syntax::{
        ParsedFunctionType, ParsedInterfaceMember, ParsedTypeParameter, TextSpan,
    };
    use surge_ts_types::{
        FunctionType, ObjectProperty, ObjectType, PropertyMap, ResolveReference, Type,
        TypeReference, union_type,
    };

    use super::*;
    use crate::context::{CheckerContext, CheckerOptions, FileKind};
    use crate::symbols::{InterfaceInfo, merge_interface_infos};

    const LIB_DOM: &str = "/typescript/lib/lib.dom.d.ts";

    fn interface(name: &str, start: usize, type_parameters: &[&str]) -> InterfaceInfo {
        InterfaceInfo::new(
            name.to_string(),
            LIB_DOM.to_string(),
            Some(TextSpan {
                start,
                end: start + name.len(),
            }),
            type_parameters
                .iter()
                .map(|name| ParsedTypeParameter {
                    name: (*name).to_string(),
                    name_span: None,
                    constraint: None,
                    default_type: None,
                    span: None,
                    is_const: false,
                    is_in: false,
                    is_out: false,
                })
                .collect(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
        )
    }

    fn context(options: CheckerOptions) -> CheckerContext {
        CheckerContext::new(
            LIB_DOM.to_string(),
            options,
            surge_ts_types::fx::FxHashMap::from_iter([(
                LIB_DOM.to_string(),
                FileKind::PhysicalDefaultLib,
            )]),
        )
    }

    fn interface_with_methods(
        name: &str,
        start: usize,
        type_parameters: &[&str],
        methods: &[(&str, usize)],
    ) -> InterfaceInfo {
        let mut interface = interface(name, start, type_parameters);
        let body = Arc::make_mut(&mut interface.body);
        body.members = methods
            .iter()
            .map(|(name, start)| ParsedInterfaceMember {
                name: (*name).to_string(),
                name_span: Some(TextSpan {
                    start: *start,
                    end: *start + name.len(),
                }),
                signature_span: None,
                written_name: None,
                optional: false,
                is_abstract: false,
                is_method: true,
                readonly: false,
                write_ty: None,
                ty: ParsedType::Function(std::sync::Arc::new(ParsedFunctionType {
                    parameters: Vec::new(),
                    return_type: Box::new(ParsedType::String),
                    type_parameters: Vec::new(),
                })),
            })
            .collect();
        body.member_fragments = vec![body.declaration_fragments[0].clone(); body.members.len()];
        interface
    }

    #[test]
    fn physical_lib_interface_cache_basic() {
        let interface = interface("Body", 100, &[]);
        let ctx = context(CheckerOptions::default());
        let key = canonical_physical_interface_key(
            &interface,
            &TypeParameterSubstitution::new(),
            &ctx,
            false,
        )
        .unwrap();

        assert_eq!(ctx.substitution_store.stats().stored_arguments, 0);
        assert_eq!(key.declaration.declaration_start, 100);
        assert_eq!(&*key.declaration.declaration_name, "Body");
    }

    #[test]
    fn physical_lib_interface_cache_generic_same_args_many_consumers() {
        let interface = interface("Iterator", 200, &["T"]);
        let ctx = context(CheckerOptions::default());
        let mut substitution = TypeParameterSubstitution::new();
        substitution.insert("T".to_string(), Type::String);
        let key = canonical_physical_interface_key(&interface, &substitution, &ctx, false).unwrap();

        let first = intern_physical_interface_instantiation(&ctx, key.clone(), Type::String);
        let second = intern_physical_interface_instantiation(&ctx, key.clone(), Type::Number);
        let lookup = lookup_physical_interface_instantiation(&ctx, &key).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&first, &lookup));
        assert_eq!(*lookup, Type::String);
    }

    #[test]
    fn physical_lib_interface_cache_different_args() {
        let interface = interface("Iterator", 200, &["T"]);
        let ctx = context(CheckerOptions::default());
        let mut strings = TypeParameterSubstitution::new();
        strings.insert("T".to_string(), Type::String);
        let mut numbers = TypeParameterSubstitution::new();
        numbers.insert("T".to_string(), Type::Number);

        let string_key =
            canonical_physical_interface_key(&interface, &strings, &ctx, false).unwrap();
        let number_key =
            canonical_physical_interface_key(&interface, &numbers, &ctx, false).unwrap();

        assert_ne!(string_key, number_key);
    }

    #[test]
    fn physical_lib_interface_cache_degraded_not_cacheable() {
        let interface = interface("Iterator", 200, &["T"]);
        let ctx = context(CheckerOptions::default());
        let mut unknown = TypeParameterSubstitution::new();
        unknown.insert("T".to_string(), Type::Unknown);
        let mut placeholder = TypeParameterSubstitution::new();
        placeholder.insert_placeholder("T".to_string(), Type::String);

        assert_eq!(
            canonical_physical_interface_key(&interface, &unknown, &ctx, false),
            Err(InterfaceCacheSkipReason::UnresolvedTypeArgument)
        );
        assert_eq!(
            canonical_physical_interface_key(&interface, &placeholder, &ctx, false),
            Err(InterfaceCacheSkipReason::UnresolvedTypeArgument)
        );
    }

    #[test]
    fn physical_lib_interface_cache_interface_merge_identity() {
        let original = interface("Element", 300, &[]);
        let augmentation = interface("Element", 900, &[]);
        let merged = merge_interface_infos(&original, &augmentation);

        let original_id = stable_interface_declaration_id(&original).unwrap();
        let merged_id = stable_interface_declaration_id(&merged).unwrap();

        assert_ne!(original_id, merged_id);
        assert_eq!(merged_id.merged_fragments.len(), 2);
        assert_eq!(merged_id.merged_fragments[0].declaration_start, 300);
        assert_eq!(merged_id.merged_fragments[1].declaration_start, 900);
    }

    #[test]
    fn physical_lib_interface_cache_environment_identity() {
        let interface = interface("Element", 300, &[]);
        let default_ctx = context(CheckerOptions::default());
        let mut skip_lib_options = CheckerOptions::default();
        skip_lib_options.skip_lib_check = true;
        let skip_lib_ctx = context(skip_lib_options);

        let default_key = canonical_physical_interface_key(
            &interface,
            &TypeParameterSubstitution::new(),
            &default_ctx,
            false,
        )
        .unwrap();
        let skip_lib_key = canonical_physical_interface_key(
            &interface,
            &TypeParameterSubstitution::new(),
            &skip_lib_ctx,
            false,
        )
        .unwrap();

        assert_ne!(default_key, skip_lib_key);
    }

    #[test]
    fn physical_lib_interface_cache_rejects_unknown_and_context() {
        struct ContextualResolver;

        impl ResolveReference for ContextualResolver {
            fn resolve(&self) -> Type {
                Type::String
            }

            fn retains_resolution_context(&self) -> bool {
                true
            }
        }

        let contextual = Type::Reference(TypeReference::new(
            "lib.dom.d.ts\0Contextual",
            "Contextual",
            Vec::<Type>::new(),
            Arc::new(ContextualResolver),
        ));

        assert_eq!(
            validate_physical_interface_cache_value(&Type::Unknown),
            Err(InterfaceCacheValueRejection::Unknown)
        );
        assert_eq!(
            validate_physical_interface_cache_value(&contextual),
            Err(InterfaceCacheValueRejection::ResolutionContext)
        );
        assert!(validate_physical_interface_cache_value(&Type::GenuineUnknown).is_ok());
        let contextual_method = Type::Function(FunctionType::new(
            vec![Type::Function(FunctionType::new(
                vec![Type::String],
                Type::Void,
                false,
                1,
            ))],
            Type::Void,
            false,
            1,
        ));
        assert!(physical_interface_method_has_contextual_typing_dependency(
            &contextual_method
        ));
    }

    #[test]
    fn physical_lib_interface_cache_preserves_overloads_and_signatures() {
        let interface = interface("Callable", 1_000, &[]);
        let ctx = context(CheckerOptions::default());
        let key = canonical_physical_interface_key(
            &interface,
            &TypeParameterSubstitution::new(),
            &ctx,
            false,
        )
        .unwrap();
        let first_overload = Type::Function(FunctionType::new(
            vec![Type::String],
            Type::Number,
            false,
            1,
        ));
        let second_overload = Type::Function(FunctionType::new(
            vec![Type::Number],
            Type::String,
            false,
            1,
        ));
        let mut properties = PropertyMap::default();
        properties.insert(
            "method".into(),
            ObjectProperty::required(union_type(vec![first_overload, second_overload])),
        );
        let object = ObjectType::new(properties, Some(Type::String))
            .with_call_signature(FunctionType::new(Vec::new(), Type::Boolean, false, 0))
            .with_construct_signature(FunctionType::new(Vec::new(), Type::Any, false, 0));

        let cached =
            intern_physical_interface_instantiation(&ctx, key.clone(), Type::Object(object));
        let lookup = lookup_physical_interface_instantiation(&ctx, &key).unwrap();
        assert!(Arc::ptr_eq(&cached, &lookup));
        let Type::Object(object) = &*lookup else {
            panic!("cached interface must remain an object");
        };
        let Type::Union(overloads) = &object.get_property("method").unwrap().ty else {
            panic!("method overload array must remain ordered");
        };
        assert_eq!(overloads.types()[0].name(), "(string) => number");
        assert_eq!(overloads.types()[1].name(), "(number) => string");
        assert_eq!(
            object.call_signature().unwrap().return_type(),
            &Type::Boolean
        );
        assert_eq!(
            object.construct_signature().unwrap().return_type(),
            &Type::Any
        );
        assert_eq!(object.string_index_type.as_deref(), Some(&Type::String));
    }

    #[test]
    fn physical_lib_interface_cache_recursive_reference_is_not_peeled() {
        struct RecursiveResolver;

        impl ResolveReference for RecursiveResolver {
            fn resolve(&self) -> Type {
                panic!("cache identity and eligibility must not peel references")
            }
        }

        let recursive = Type::Reference(TypeReference::new(
            "lib.es2015.iterable.d.ts\0IterableIterator",
            "IterableIterator<string>",
            vec![Type::String],
            Arc::new(RecursiveResolver),
        ));
        assert!(validate_physical_interface_cache_value(&recursive).is_ok());

        let interface = interface("IterableIterator", 1_100, &["T"]);
        let ctx = context(CheckerOptions::default());
        let mut substitution = TypeParameterSubstitution::new();
        substitution.insert("T".to_string(), recursive);
        let _key =
            canonical_physical_interface_key(&interface, &substitution, &ctx, false).unwrap();

        assert_eq!(ctx.substitution_store.stats().stored_arguments, 1);
    }

    #[test]
    fn physical_lib_method_cache_uses_stable_member_and_substitution_identity() {
        let interface = interface_with_methods("Iterator", 2_000, &["T"], &[("next", 2_010)]);
        let ctx = context(CheckerOptions::default());
        let mut substitution = TypeParameterSubstitution::new();
        substitution.insert("T".to_string(), Type::String);
        let interface_key =
            canonical_physical_interface_key(&interface, &substitution, &ctx, false).unwrap();
        let template =
            physical_interface_declaration_template(&ctx, &interface, &interface_key.declaration)
                .unwrap();
        let key =
            interface_member_instantiation_key(&template.members[0].declaration, &interface_key);
        let first = intern_physical_interface_method(
            &ctx,
            key.clone(),
            FunctionType::new(Vec::new(), Type::String, false, 0),
        );
        let second = intern_physical_interface_method(
            &ctx,
            key.clone(),
            FunctionType::new(Vec::new(), Type::Number, false, 0),
        );

        assert!(std::ptr::eq(first.payload(), second.payload()));
        assert!(std::ptr::eq(
            first.payload(),
            lookup_physical_interface_method(&ctx, &key)
                .unwrap()
                .payload()
        ));

        let mut number_substitution = TypeParameterSubstitution::new();
        number_substitution.insert("T".to_string(), Type::Number);
        let number_interface_key =
            canonical_physical_interface_key(&interface, &number_substitution, &ctx, false)
                .unwrap();
        let number_key = interface_member_instantiation_key(
            &template.members[0].declaration,
            &number_interface_key,
        );
        assert_ne!(key, number_key);
        assert!(lookup_physical_interface_method(&ctx, &number_key).is_none());
    }

    #[test]
    fn physical_lib_overload_cache_preserves_declaration_order_and_duplicates() {
        let interface = interface_with_methods(
            "Headers",
            3_000,
            &[],
            &[("append", 3_010), ("append", 3_020), ("append", 3_030)],
        );
        let ctx = context(CheckerOptions::default());
        let interface_key = canonical_physical_interface_key(
            &interface,
            &TypeParameterSubstitution::new(),
            &ctx,
            false,
        )
        .unwrap();
        let template =
            physical_interface_declaration_template(&ctx, &interface, &interface_key.declaration)
                .unwrap();
        let group = &template.method_groups[0];

        assert_eq!(group.ordered_members.len(), 3);
        assert_eq!(group.ordered_members[0].declaration_start, 3_010);
        assert_eq!(group.ordered_members[1].declaration_start, 3_020);
        assert_eq!(group.ordered_members[2].declaration_start, 3_030);
        assert_eq!(group.ordered_members[0].overload_index, 0);
        assert_eq!(group.ordered_members[1].overload_index, 1);
        assert_eq!(group.ordered_members[2].overload_index, 2);

        let key = interface_overload_instantiation_key(
            &interface_key.declaration,
            group,
            3,
            &interface_key,
        );
        let first = intern_physical_interface_overload(
            &ctx,
            key.clone(),
            FunctionType::new(vec![Type::String], Type::String, false, 1),
        );
        let second = lookup_physical_interface_overload(&ctx, &key).unwrap();
        assert!(std::ptr::eq(first.payload(), second.payload()));
    }
}
