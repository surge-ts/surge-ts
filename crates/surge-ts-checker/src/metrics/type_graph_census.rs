use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::sync::Arc;

use surge_ts_types::{
    FunctionType, FunctionTypePayload, ObjectProperty, ObjectType, ProgramTypeStore, PropertyMap,
    Type, TypeReference, UnionTypePayload,
};

use crate::context::{CheckerContext, DeclarationResolutionState};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CensusExternalRetention {
    pub(crate) ast_nodes: u64,
    pub(crate) ast_estimated_bytes: u64,
    pub(crate) source_text_bytes: u64,
    pub(crate) module_analysis_entries: u64,
    pub(crate) module_analysis_estimated_bytes: u64,
    pub(crate) symbol_entries: u64,
    pub(crate) symbol_estimated_bytes: u64,
}

#[derive(Default)]
struct TypeGraphCensus {
    function_payloads: HashSet<usize>,
    parameter_lists: HashSet<usize>,
    object_property_maps: HashSet<usize>,
    union_payloads: HashSet<usize>,
    union_member_lists: HashSet<usize>,
    reference_resolvers: HashSet<usize>,
    root_arcs: HashSet<usize>,
    function_payload_bytes: u64,
    parameter_list_bytes: u64,
    function_return_graph_bytes: u64,
    object_payload_count: u64,
    object_payload_bytes: u64,
    property_map_bytes: u64,
    method_overload_bytes: u64,
    union_payload_bytes: u64,
    union_member_bytes: u64,
    type_reference_count: u64,
    type_reference_bytes: u64,
    lazy_resolver_bytes: u64,
    context_retaining_resolvers: u64,
    checker_context_snapshot_bytes: u64,
    generic_cache_bytes: u64,
    instantiation_cache_bytes: u64,
    top_identities: HashMap<String, (u64, u64)>,
}

impl TypeGraphCensus {
    fn walk_type(&mut self, ty: &Type) {
        match ty {
            Type::Function(function) => self.walk_function(function),
            Type::Object(object) => self.walk_object(object),
            Type::Array(element) => self.walk_type(element),
            Type::Tuple(elements) => {
                for element in elements {
                    self.walk_type(element);
                }
            }
            Type::OpenTuple(tuple) => {
                for element in tuple.leading.iter().chain(tuple.trailing.iter()) {
                    self.walk_type(element);
                }
                self.walk_type(&tuple.rest);
            }
            Type::Union(union) => {
                if self.union_payloads.insert(union.payload_address()) {
                    self.union_payload_bytes += size_of::<UnionTypePayload>() as u64;
                    if self.union_member_lists.insert(union.member_list_address()) {
                        self.union_member_bytes += (union.types().len() * size_of::<Type>()) as u64;
                        for member in union.types() {
                            self.walk_type(member);
                        }
                    }
                }
            }
            Type::Reference(reference) => self.walk_reference(reference),
            Type::ErrorType(_)
            | Type::String
            | Type::Number
            | Type::Boolean
            | Type::BigInt
            | Type::Symbol
            | Type::Undefined
            | Type::Null
            | Type::Void
            | Type::Any
            | Type::Unknown
            | Type::TypeParameter(_)
            | Type::GenuineUnknown
            | Type::Never
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_) => {}
        }
    }

    fn walk_root_arc(&mut self, ty: &Arc<Type>) {
        if self.root_arcs.insert(Arc::as_ptr(ty) as usize) {
            self.walk_type(ty);
        }
    }

    fn walk_function(&mut self, function: &FunctionType) {
        if !self.function_payloads.insert(function.payload_address()) {
            return;
        }
        self.function_payload_bytes += size_of::<FunctionTypePayload>() as u64;
        let parameters = function.parameters();
        if self
            .parameter_lists
            .insert(function.parameter_list_address())
        {
            self.parameter_list_bytes += (parameters.len() * size_of::<Type>()) as u64;
            for parameter in parameters {
                self.walk_type(parameter);
            }
        }
        let before = self.attributed_type_bytes();
        self.walk_type(function.return_type());
        self.function_return_graph_bytes += self.attributed_type_bytes().saturating_sub(before);
    }

    fn walk_object(&mut self, object: &ObjectType) {
        self.object_payload_count += 1;
        self.object_payload_bytes += size_of::<ObjectType>() as u64;
        let property_map_address = Arc::as_ptr(&object.properties) as usize;
        if self.object_property_maps.insert(property_map_address) {
            self.property_map_bytes += size_of::<PropertyMap>() as u64;
            for (name, property) in object.properties.iter() {
                self.property_map_bytes +=
                    (size_of::<Arc<str>>() + name.len() + size_of::<ObjectProperty>()) as u64;
                self.walk_type(&property.ty);
            }
        }
        if let Some(index) = object.string_index_type.as_deref() {
            self.walk_type(index);
        }
        if let Some(call) = object.call_signature() {
            self.method_overload_bytes += size_of::<FunctionType>() as u64;
            self.walk_function(call);
        }
        if let Some(construct) = object.construct_signature() {
            self.method_overload_bytes += size_of::<FunctionType>() as u64;
            self.walk_function(construct);
        }
        if let Some(identity) = object.alias_id.as_deref() {
            self.record_identity(identity, size_of::<ObjectType>() as u64);
        }
    }

    fn walk_reference(&mut self, reference: &TypeReference) {
        self.type_reference_count += 1;
        let shallow = size_of::<TypeReference>()
            + reference.id.len()
            + reference.display.len()
            + reference.arguments.len() * size_of::<Type>();
        self.type_reference_bytes += shallow as u64;
        if self
            .reference_resolvers
            .insert(reference.resolver_address())
        {
            self.lazy_resolver_bytes +=
                size_of::<Arc<dyn surge_ts_types::ResolveReference>>() as u64;
            if reference.retains_resolution_context() {
                self.context_retaining_resolvers += 1;
                self.checker_context_snapshot_bytes += size_of::<CheckerContext>() as u64;
            }
        }
        self.record_identity(&reference.id, shallow as u64);
        for argument in reference.arguments.iter() {
            self.walk_type(argument);
        }
    }

    fn record_identity(&mut self, identity: &str, bytes: u64) {
        let entry = self.top_identities.entry(identity.to_string()).or_default();
        entry.0 += 1;
        entry.1 += bytes;
    }

    fn attributed_type_bytes(&self) -> u64 {
        self.function_payload_bytes
            + self.parameter_list_bytes
            + self.object_payload_bytes
            + self.property_map_bytes
            + self.method_overload_bytes
            + self.union_payload_bytes
            + self.union_member_bytes
            + self.type_reference_bytes
            + self.lazy_resolver_bytes
            + self.checker_context_snapshot_bytes
    }
}

pub(crate) fn type_graph_census_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_TYPE_GRAPH_CENSUS").is_some())
}

pub(crate) fn emit_type_graph_census(
    stage: &str,
    ctx: Option<&CheckerContext>,
    store: &Arc<ProgramTypeStore>,
    external: CensusExternalRetention,
) {
    if !type_graph_census_enabled() {
        return;
    }

    let mut census = TypeGraphCensus::default();
    if let Some(ctx) = ctx {
        if let Ok(cache) = ctx.resolved_named_types.lock() {
            for state in cache.values() {
                if let DeclarationResolutionState::Resolved { ty, .. } = state {
                    census.walk_type(ty);
                }
            }
        }
        if let Ok(cache) = ctx.program_resolved_generic_types.lock() {
            census.generic_cache_bytes +=
                (cache.len() * size_of::<crate::context::DeclarationResolutionKey>()) as u64;
            for bucket in cache.values() {
                census.generic_cache_bytes += (bucket.len()
                    * size_of::<crate::context::GenericInstantiationCacheEntry>())
                    as u64;
                for entry in bucket {
                    for argument in &entry.arguments {
                        census.walk_type(argument);
                    }
                    census.walk_type(&entry.ty);
                }
            }
        }
        if let Ok(cache) = ctx.program_instantiations.lock() {
            census.instantiation_cache_bytes +=
                (cache.len() * size_of::<crate::context::DeclarationResolutionKey>()) as u64;
            for bucket in cache.values() {
                census.instantiation_cache_bytes +=
                    (bucket.len() * size_of::<crate::context::InstantiationCacheEntry>()) as u64;
                for entry in bucket {
                    for argument in &entry.arguments {
                        census.walk_type(argument);
                    }
                    census.walk_root_arc(&entry.resolved);
                }
            }
        }
        if let Ok(cache) = ctx.physical_interface_instantiations.lock() {
            for ty in cache.values() {
                census.walk_root_arc(ty);
            }
        }
        if let Ok(cache) = ctx.physical_interface_method_instantiations.lock() {
            for function in cache.values() {
                census.walk_function(function);
            }
        }
        if let Ok(cache) = ctx.physical_interface_overload_instantiations.lock() {
            for function in cache.values() {
                census.walk_function(function);
            }
        }
    }

    let store_stats = store.stats();
    let declaration_environment_stats = ctx
        .map(|ctx| ctx.declaration_environment_store.stats())
        .unwrap_or_default();
    let substitution_store_stats = ctx
        .map(|ctx| ctx.substitution_store.stats())
        .unwrap_or_default();
    let attributed = census.attributed_type_bytes()
        + census.generic_cache_bytes
        + census.instantiation_cache_bytes
        + external.ast_estimated_bytes
        + external.source_text_bytes
        + external.module_analysis_estimated_bytes
        + external.symbol_estimated_bytes;
    let footprint = super::current_footprint_bytes();
    let allocator_unattributed = footprint.map(|bytes| bytes.saturating_sub(attributed));
    let mut top = census.top_identities.into_iter().collect::<Vec<_>>();
    top.sort_by_key(|(_, (_, bytes))| std::cmp::Reverse(*bytes));
    top.truncate(12);
    let top = top
        .into_iter()
        .map(|(identity, (count, bytes))| {
            format!(
                "{{\"identity\":\"{}\",\"count\":{count},\"retainedBytes\":{bytes}}}",
                json_escape(&identity)
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    eprintln!(
        "{{\"typeGraphCensus\":true,\"stage\":\"{}\",\"footprintBytes\":{},\
         \"allocatorVisibleUnattributedBytes\":{},\"functionPayloads\":{{\"unique\":{},\
         \"shallowBytes\":{}}},\"functionParameterLists\":{{\"unique\":{},\"ownedBytes\":{}}},\
         \"functionReturnGraphs\":{{\"estimatedSharedBytes\":{}}},\"objectPayloads\":{{\"count\":{},\
         \"shallowBytes\":{}}},\"propertyMaps\":{{\"unique\":{},\"ownedBytes\":{}}},\
         \"methodOverloadStorageBytes\":{},\"unionPayloads\":{{\"unique\":{},\"shallowBytes\":{}}},\
         \"unionMemberArrays\":{{\"unique\":{},\"ownedBytes\":{}}},\
         \"intersectionPayloadBytes\":0,\"typeReferences\":{{\"count\":{},\"ownedBytes\":{}}},\
         \"lazyResolvers\":{{\"unique\":{},\"shallowBytes\":{}}},\
         \"checkerContextSnapshots\":{{\"uniqueRetainingResolvers\":{},\"estimatedBytes\":{}}},\
         \"declarationEnvironments\":{{\"requests\":{},\"hits\":{},\"unique\":{}}},\
         \"substitutions\":{{\"requests\":{},\"hits\":{},\"unique\":{},\"inputArguments\":{},\
         \"storedArguments\":{},\"argumentStorageAvoided\":{}}},\
         \"genericCacheBytes\":{},\"instantiationCacheBytes\":{},\
         \"moduleAnalysis\":{{\"entries\":{},\"estimatedBytes\":{}}},\
         \"symbolsAndScopes\":{{\"entries\":{},\"estimatedBytes\":{}}},\
         \"astAndSourceText\":{{\"nodes\":{},\"astBytes\":{},\"sourceTextBytes\":{}}},\
         \"canonicalStore\":{{\"functionRequests\":{},\"uniqueFunctionIds\":{},\"functionHits\":{},\
         \"functionFallbacks\":{},\"overloadMergeRequests\":{},\"overloadMergeHits\":{},\
         \"overloadMergeMisses\":{},\"parameterListIds\":{},\"parameterElementsAvoided\":{},\
         \"unionIds\":{},\"unionHits\":{},\"unionMembersAvoided\":{},\"propertyMapIds\":{},\
         \"propertyMapHits\":{},\"propertyEntriesAvoided\":{},\"lockContentions\":{}}},\
         \"topIdentities\":[{}]}}",
        json_escape(stage),
        super::json_u64_opt(footprint),
        super::json_u64_opt(allocator_unattributed),
        census.function_payloads.len(),
        census.function_payload_bytes,
        census.parameter_lists.len(),
        census.parameter_list_bytes,
        census.function_return_graph_bytes,
        census.object_payload_count,
        census.object_payload_bytes,
        census.object_property_maps.len(),
        census.property_map_bytes,
        census.method_overload_bytes,
        census.union_payloads.len(),
        census.union_payload_bytes,
        census.union_member_lists.len(),
        census.union_member_bytes,
        census.type_reference_count,
        census.type_reference_bytes,
        census.reference_resolvers.len(),
        census.lazy_resolver_bytes,
        census.context_retaining_resolvers,
        census.checker_context_snapshot_bytes,
        declaration_environment_stats.0,
        declaration_environment_stats.1,
        declaration_environment_stats.2,
        substitution_store_stats.requests,
        substitution_store_stats.hits,
        substitution_store_stats.unique,
        substitution_store_stats.input_arguments,
        substitution_store_stats.stored_arguments,
        substitution_store_stats.argument_storage_avoided,
        census.generic_cache_bytes,
        census.instantiation_cache_bytes,
        external.module_analysis_entries,
        external.module_analysis_estimated_bytes,
        external.symbol_entries,
        external.symbol_estimated_bytes,
        external.ast_nodes,
        external.ast_estimated_bytes,
        external.source_text_bytes,
        store_stats.function_requests,
        store_stats.function_misses,
        store_stats.function_hits,
        store_stats.function_fallbacks,
        store_stats.overload_merge_requests,
        store_stats.overload_merge_hits,
        store_stats.overload_merge_misses,
        store_stats.parameter_list_misses,
        store_stats.parameter_list_elements_avoided,
        store_stats.union_misses,
        store_stats.union_hits,
        store_stats.union_member_elements_avoided,
        store_stats.property_map_misses,
        store_stats.property_map_hits,
        store_stats.property_entries_avoided,
        store_stats.interner_lock_contentions,
        top,
    );
}

pub(crate) fn render_program_type_store_stats(
    store: &ProgramTypeStore,
    declaration_environment_stats: (u64, u64, u64),
    substitution_store_stats: crate::context::SubstitutionStoreStats,
) {
    let stats = store.stats();
    eprintln!(
        "  canonical_type_store: function_requests={} unique_function_ids={} function_hits={} \
         function_fallbacks={} overload_merge_requests={} overload_merge_hits={} \
         overload_merge_misses={} parameter_list_ids={} parameter_list_hits={} \
         parameter_elements_avoided={} union_ids={} union_hits={} union_members_avoided={} \
         property_map_ids={} property_map_hits={} property_entries_avoided={} lock_contentions={}",
        stats.function_requests,
        stats.function_misses,
        stats.function_hits,
        stats.function_fallbacks,
        stats.overload_merge_requests,
        stats.overload_merge_hits,
        stats.overload_merge_misses,
        stats.parameter_list_misses,
        stats.parameter_list_hits,
        stats.parameter_list_elements_avoided,
        stats.union_misses,
        stats.union_hits,
        stats.union_member_elements_avoided,
        stats.property_map_misses,
        stats.property_map_hits,
        stats.property_entries_avoided,
        stats.interner_lock_contentions,
    );
    eprintln!(
        "  declaration_environment_store: requests={} hits={} unique={}",
        declaration_environment_stats.0,
        declaration_environment_stats.1,
        declaration_environment_stats.2,
    );
    eprintln!(
        "  substitution_store: requests={} hits={} unique={} input_arguments={} \
         stored_arguments={} argument_storage_avoided={}",
        substitution_store_stats.requests,
        substitution_store_stats.hits,
        substitution_store_stats.unique,
        substitution_store_stats.input_arguments,
        substitution_store_stats.stored_arguments,
        substitution_store_stats.argument_storage_avoided,
    );
    if super::rss_json_enabled() {
        eprintln!(
            "{{\"canonicalTypeStore\":{{\"functionRequests\":{},\"uniqueFunctionIds\":{},\
             \"functionHits\":{},\"functionFallbacks\":{},\"overloadMergeRequests\":{},\
             \"overloadMergeHits\":{},\"overloadMergeMisses\":{},\"parameterListIds\":{},\
             \"parameterListHits\":{},\"parameterElementsAvoided\":{},\"unionIds\":{},\
             \"unionHits\":{},\"unionMembersAvoided\":{},\"propertyMapIds\":{},\
             \"propertyMapHits\":{},\"propertyEntriesAvoided\":{},\"lockContentions\":{}}}}}",
            stats.function_requests,
            stats.function_misses,
            stats.function_hits,
            stats.function_fallbacks,
            stats.overload_merge_requests,
            stats.overload_merge_hits,
            stats.overload_merge_misses,
            stats.parameter_list_misses,
            stats.parameter_list_hits,
            stats.parameter_list_elements_avoided,
            stats.union_misses,
            stats.union_hits,
            stats.union_member_elements_avoided,
            stats.property_map_misses,
            stats.property_map_hits,
            stats.property_entries_avoided,
            stats.interner_lock_contentions,
        );
        eprintln!(
            "{{\"declarationEnvironmentStore\":{{\"requests\":{},\"hits\":{},\"unique\":{}}}}}",
            declaration_environment_stats.0,
            declaration_environment_stats.1,
            declaration_environment_stats.2,
        );
        eprintln!(
            "{{\"substitutionStore\":{{\"requests\":{},\"hits\":{},\"unique\":{},\
             \"inputArguments\":{},\"storedArguments\":{},\"argumentStorageAvoided\":{}}}}}",
            substitution_store_stats.requests,
            substitution_store_stats.hits,
            substitution_store_stats.unique,
            substitution_store_stats.input_arguments,
            substitution_store_stats.stored_arguments,
            substitution_store_stats.argument_storage_avoided,
        );
    }
}

fn json_escape(value: &str) -> String {
    use std::fmt::Write;

    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            control if control <= '\u{1f}' => {
                write!(&mut escaped, "\\u{:04x}", control as u32).unwrap();
            }
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::json_escape;

    #[test]
    fn census_identity_is_valid_json_text() {
        assert_eq!(
            json_escape("file.ts\0Type\n\""),
            "file.ts\\u0000Type\\n\\\""
        );
    }
}
