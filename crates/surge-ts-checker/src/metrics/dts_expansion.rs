use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use surge_ts_types::{snapshot_function_type_counters, snapshot_union_type_counters};

use super::{current_footprint_bytes, peak_footprint_bytes, snapshot_program_counters};
use crate::context::{
    DeclarationResolutionKey, InterfaceInstantiationKey, InterfaceMemberInstantiationKey,
    InterfaceOverloadInstantiationKey, StableInterfaceDeclarationId,
    StableInterfaceMemberDeclarationId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub(crate) enum InterfaceDegradationReason {
    MissingImportedBinding,
    UnresolvedTypeArgument,
    UnsupportedCanonicalArgument,
    RecursivePlaceholder,
    UnknownFallback,
    GenuineUnknown,
    ContextRetainingReference,
    LazyCheckerContextReference,
    UnsupportedComputedProperty,
    UnsupportedSymbolProperty,
    PartialGenericSubstitution,
    HeritageResolutionFailure,
    MemberAnnotationFailure,
    MethodSignatureFailure,
    IndexSignatureFailure,
    ContextualTypingDependency,
    UtilityFallback,
    DiagnosticProduced,
    HadErrorFlag,
    TraversalLimit,
    Other,
}

impl InterfaceDegradationReason {
    fn label(self) -> &'static str {
        match self {
            Self::MissingImportedBinding => "missing_imported_binding",
            Self::UnresolvedTypeArgument => "unresolved_type_argument",
            Self::UnsupportedCanonicalArgument => "unsupported_canonical_argument",
            Self::RecursivePlaceholder => "recursive_placeholder",
            Self::UnknownFallback => "unknown_fallback",
            Self::GenuineUnknown => "genuine_unknown",
            Self::ContextRetainingReference => "context_retaining_reference",
            Self::LazyCheckerContextReference => "lazy_checker_context_reference",
            Self::UnsupportedComputedProperty => "unsupported_computed_property",
            Self::UnsupportedSymbolProperty => "unsupported_symbol_property",
            Self::PartialGenericSubstitution => "partial_generic_substitution",
            Self::HeritageResolutionFailure => "heritage_resolution_failure",
            Self::MemberAnnotationFailure => "member_annotation_failure",
            Self::MethodSignatureFailure => "method_signature_failure",
            Self::IndexSignatureFailure => "index_signature_failure",
            Self::ContextualTypingDependency => "contextual_typing_dependency",
            Self::UtilityFallback => "utility_fallback",
            Self::DiagnosticProduced => "diagnostic_produced",
            Self::HadErrorFlag => "had_error_flag",
            Self::TraversalLimit => "traversal_limit",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub(crate) enum DtsExpansionReason {
    SignatureParameter,
    SignatureReturn,
    SignatureThisParameter,
    SignatureTypePredicate,
    GenericConstraint,
    GenericDefault,
    CallSignature,
    ConstructSignature,
    InterfaceMethodMapping,
    ClassMethod,
    ClassConstructor,
    FunctionTypeAnnotation,
    ModuleExportCollection,
    OverloadResolution,
    CallResolution,
    ConstructResolution,
    Assignability,
    ContextualTyping,
    GenericInference,
    PropertyLookup,
    IndexedAccess,
    ConditionalType,
    MappedType,
    IntersectionMerge,
    UnionNormalization,
    ApparentType,
    DiagnosticDisplay,
    ModuleDedup,
    InterfaceResolution,
    InterfaceOwnPropertyMapping,
    InterfaceCallSignatureMapping,
    InterfaceConstructSignatureMapping,
    InterfaceIndexSignatureMapping,
    InterfaceHeritageResolution,
    InheritedPropertyMerge,
    InheritedMethodMerge,
    OverloadArrayMerge,
    DefaultLibInterfaceInstantiation,
    DependencyInterfaceInstantiation,
    GenericSubstitution,
    ParsedTypeMapping,
    Other,
}

impl DtsExpansionReason {
    pub(crate) const ALL: [Self; 42] = [
        Self::SignatureParameter,
        Self::SignatureReturn,
        Self::SignatureThisParameter,
        Self::SignatureTypePredicate,
        Self::GenericConstraint,
        Self::GenericDefault,
        Self::CallSignature,
        Self::ConstructSignature,
        Self::InterfaceMethodMapping,
        Self::ClassMethod,
        Self::ClassConstructor,
        Self::FunctionTypeAnnotation,
        Self::ModuleExportCollection,
        Self::OverloadResolution,
        Self::CallResolution,
        Self::ConstructResolution,
        Self::Assignability,
        Self::ContextualTyping,
        Self::GenericInference,
        Self::PropertyLookup,
        Self::IndexedAccess,
        Self::ConditionalType,
        Self::MappedType,
        Self::IntersectionMerge,
        Self::UnionNormalization,
        Self::ApparentType,
        Self::DiagnosticDisplay,
        Self::ModuleDedup,
        Self::InterfaceResolution,
        Self::InterfaceOwnPropertyMapping,
        Self::InterfaceCallSignatureMapping,
        Self::InterfaceConstructSignatureMapping,
        Self::InterfaceIndexSignatureMapping,
        Self::InterfaceHeritageResolution,
        Self::InheritedPropertyMerge,
        Self::InheritedMethodMerge,
        Self::OverloadArrayMerge,
        Self::DefaultLibInterfaceInstantiation,
        Self::DependencyInterfaceInstantiation,
        Self::GenericSubstitution,
        Self::ParsedTypeMapping,
        Self::Other,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::SignatureParameter => "signature_parameter",
            Self::SignatureReturn => "signature_return",
            Self::SignatureThisParameter => "signature_this_parameter",
            Self::SignatureTypePredicate => "signature_type_predicate",
            Self::GenericConstraint => "generic_constraint",
            Self::GenericDefault => "generic_default",
            Self::CallSignature => "call_signature",
            Self::ConstructSignature => "construct_signature",
            Self::InterfaceMethodMapping => "interface_method_mapping",
            Self::ClassMethod => "class_method",
            Self::ClassConstructor => "class_constructor",
            Self::FunctionTypeAnnotation => "function_type_annotation",
            Self::ModuleExportCollection => "module_export_collection",
            Self::OverloadResolution => "overload_resolution",
            Self::CallResolution => "call_resolution",
            Self::ConstructResolution => "construct_resolution",
            Self::Assignability => "assignability",
            Self::ContextualTyping => "contextual_typing",
            Self::GenericInference => "generic_inference",
            Self::PropertyLookup => "property_lookup",
            Self::IndexedAccess => "indexed_access",
            Self::ConditionalType => "conditional_type",
            Self::MappedType => "mapped_type",
            Self::IntersectionMerge => "intersection_merge",
            Self::UnionNormalization => "union_normalization",
            Self::ApparentType => "apparent_type",
            Self::DiagnosticDisplay => "diagnostic_display",
            Self::ModuleDedup => "module_dedup",
            Self::InterfaceResolution => "interface_resolution",
            Self::InterfaceOwnPropertyMapping => "interface_own_property_mapping",
            Self::InterfaceCallSignatureMapping => "interface_call_signature_mapping",
            Self::InterfaceConstructSignatureMapping => "interface_construct_signature_mapping",
            Self::InterfaceIndexSignatureMapping => "interface_index_signature_mapping",
            Self::InterfaceHeritageResolution => "interface_heritage_resolution",
            Self::InheritedPropertyMerge => "inherited_property_merge",
            Self::InheritedMethodMerge => "inherited_method_merge",
            Self::OverloadArrayMerge => "overload_array_merge",
            Self::DefaultLibInterfaceInstantiation => "default_lib_interface_instantiation",
            Self::DependencyInterfaceInstantiation => "dependency_interface_instantiation",
            Self::GenericSubstitution => "generic_substitution",
            Self::ParsedTypeMapping => "parsed_type_mapping",
            Self::Other => "other",
        }
    }
}

thread_local! {
    static CURRENT_REASON: Cell<DtsExpansionReason> = const { Cell::new(DtsExpansionReason::Other) };
    static EXPANSION_DEGRADATION_EPOCH: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn with_dts_expansion_reason<T>(reason: DtsExpansionReason, f: impl FnOnce() -> T) -> T {
    CURRENT_REASON.with(|current| {
        let previous = current.replace(reason);
        let previous_function_reason =
            surge_ts_types::replace_function_type_expansion_reason(reason as usize);
        let result = f();
        surge_ts_types::replace_function_type_expansion_reason(previous_function_reason);
        current.set(previous);
        result
    })
}

pub(crate) fn current_dts_expansion_reason() -> DtsExpansionReason {
    CURRENT_REASON.get()
}

pub(crate) fn note_expansion_degradation() {
    EXPANSION_DEGRADATION_EPOCH.with(|epoch| epoch.set(epoch.get().wrapping_add(1)));
}

pub(crate) fn expansion_degradation_epoch() -> u64 {
    EXPANSION_DEGRADATION_EPOCH.get()
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TypeCreationSnapshot {
    object_types: u64,
    function_types: u64,
    union_types: u64,
}

pub(crate) fn type_creation_snapshot() -> TypeCreationSnapshot {
    // Every consumer of this snapshot only ever feeds a `record_program_counter`
    // closure or the d.ts expansion trace, both of which are gated. Taking it
    // unconditionally locked the global counter mutex and copied the whole
    // `ProgramCounters` struct once per interface resolution (672k times on
    // tanstack-query), which profiled at ~7% of the run with counters off.
    if !super::counters_enabled() && !dts_expansion_trace_enabled() {
        return TypeCreationSnapshot {
            object_types: 0,
            function_types: 0,
            union_types: 0,
        };
    }
    let program = snapshot_program_counters();
    let functions = snapshot_function_type_counters();
    let unions = snapshot_union_type_counters();
    TypeCreationSnapshot {
        object_types: program.object_type_payload_alloc_count,
        function_types: functions.function_type_payload_alloc_count,
        union_types: unions.union_type_payload_alloc_count,
    }
}

pub(crate) fn record_interface_resolution_attempt() {
    crate::program::record_program_counter(|c| c.interface_resolution_attempt_count += 1);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_interface_resolution_result(
    declaration: Option<StableInterfaceDeclarationId>,
    key: Option<&InterfaceInstantiationKey>,
    clean: bool,
    cache_hit: bool,
    inherited_interfaces: usize,
    member_count: usize,
    before: TypeCreationSnapshot,
) {
    let after = type_creation_snapshot();
    let objects = after.object_types.saturating_sub(before.object_types);
    let functions = after.function_types.saturating_sub(before.function_types);
    let unions = after.union_types.saturating_sub(before.union_types);
    crate::program::record_program_counter(|c| {
        if clean {
            c.interface_resolution_success_count += 1;
        } else {
            c.interface_resolution_degraded_count += 1;
        }
        if cache_hit && inherited_interfaces != 0 {
            c.inherited_member_merge_cache_hit_count += inherited_interfaces as u64;
        }
    });

    let Some(declaration) = declaration else {
        return;
    };
    if !dts_expansion_trace_enabled() && !super::counters_enabled() {
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    let unique_declaration = trace
        .stable_interface_declarations
        .insert(declaration.clone());
    let mut unique_tuple = false;
    let mut duplicate_clean = false;
    let mut duplicate_degraded = false;
    if let Some(key) = key {
        let observation = trace
            .interface_instantiation_tuples
            .entry(key.clone())
            .or_default();
        unique_tuple = observation.0 == 0 && observation.1 == 0;
        if clean {
            duplicate_clean = observation.0 != 0;
            observation.0 += 1;
        } else {
            duplicate_degraded = observation.1 != 0;
            observation.1 += 1;
        }
    }
    let totals = trace.interfaces.entry(declaration).or_default();
    totals.attempts += 1;
    totals.clean_attempts += u64::from(clean);
    totals.degraded_attempts += u64::from(!clean);
    totals.unique_argument_tuples += u64::from(unique_tuple);
    totals.duplicate_clean_attempts += u64::from(duplicate_clean);
    totals.duplicate_degraded_attempts += u64::from(duplicate_degraded);
    totals.cache_hits += u64::from(cache_hit);
    totals.function_types += functions;
    totals.object_types += objects;
    totals.types += objects + functions + unions;
    totals.inherited_interfaces = inherited_interfaces;
    totals.member_count = member_count;
    drop(trace);

    crate::program::record_program_counter(|c| {
        c.unique_stable_interface_declaration_count += u64::from(unique_declaration);
        c.unique_interface_instantiation_tuple_count += u64::from(unique_tuple);
        c.duplicate_clean_interface_instantiation_count += u64::from(duplicate_clean);
        c.duplicate_degraded_interface_instantiation_count += u64::from(duplicate_degraded);
    });
}

pub(crate) fn record_inherited_member_merge(
    declaration: &StableInterfaceDeclarationId,
    base_name: &str,
    inherited_members: usize,
    inherited_methods: usize,
) {
    crate::program::record_program_counter(|c| c.inherited_member_merge_attempt_count += 1);
    if !dts_expansion_trace_enabled() {
        return;
    }
    let label = format!(
        "{}::{} <- {}",
        declaration.canonical_file, declaration.declaration_name, base_name
    );
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    *trace.inherited_merges.entry(label).or_default() += inherited_members.max(1) as u64;
    trace
        .interface_members
        .entry(declaration.clone())
        .or_default()
        .inherited_method_units += inherited_methods as u64;
}

pub(crate) fn record_interface_method_mapping(
    key: Option<&InterfaceMemberInstantiationKey>,
    clean: bool,
    reason: Option<InterfaceDegradationReason>,
) {
    crate::program::record_program_counter(|c| c.interface_method_mapping_attempt_count += 1);
    let Some(key) = key else {
        crate::program::record_program_counter(|c| c.non_reusable_interface_member_count += 1);
        return;
    };
    if !dts_expansion_trace_enabled() && !super::counters_enabled() {
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    let unique_member = trace.member_declarations.insert(key.member.clone());
    let observation = trace.method_instantiations.entry(key.clone()).or_default();
    let unique_key = observation.0 == 0 && observation.1 == 0;
    let duplicate_clean = clean && observation.0 != 0;
    let duplicate_degraded = !clean && observation.1 != 0;
    if clean {
        observation.0 += 1;
    } else {
        observation.1 += 1;
    }
    let totals = trace
        .interface_members
        .entry(key.member.containing_interface.clone())
        .or_default();
    totals.method_mapping_attempts += 1;
    totals.unique_method_keys += u64::from(unique_key);
    totals.duplicate_clean_method_mappings += u64::from(duplicate_clean);
    totals.duplicate_degraded_method_mappings += u64::from(duplicate_degraded);
    if let Some(reason) = reason {
        *trace
            .interface_degradation_reasons
            .entry((key.member.containing_interface.clone(), reason))
            .or_default() += 1;
        trace
            .first_interface_degradation
            .entry(key.member.containing_interface.clone())
            .or_insert_with(|| {
                format!(
                    "{}@{}:{} ({})",
                    key.member.declared_name,
                    key.member.canonical_file,
                    key.member.declaration_start,
                    reason.label()
                )
            });
    }
    drop(trace);
    crate::program::record_program_counter(|c| {
        c.unique_interface_member_declaration_count += u64::from(unique_member);
        c.unique_interface_method_instantiation_count += u64::from(unique_key);
        c.duplicate_clean_interface_method_mapping_count += u64::from(duplicate_clean);
        c.duplicate_degraded_interface_method_mapping_count += u64::from(duplicate_degraded);
        c.clean_reusable_interface_member_count += u64::from(clean);
        c.non_reusable_interface_member_count += u64::from(!clean);
        c.context_retaining_interface_member_count += u64::from(matches!(
            reason,
            Some(InterfaceDegradationReason::ContextRetainingReference)
                | Some(InterfaceDegradationReason::LazyCheckerContextReference)
        ));
        c.unknown_containing_interface_member_count += u64::from(matches!(
            reason,
            Some(InterfaceDegradationReason::UnknownFallback)
                | Some(InterfaceDegradationReason::GenuineUnknown)
        ));
    });
}

pub(crate) fn record_interface_overload_construction(
    key: Option<&InterfaceOverloadInstantiationKey>,
    cache_hit: bool,
) {
    crate::program::record_program_counter(|c| {
        c.interface_overload_group_construction_attempt_count += 1
    });
    let Some(key) = key else {
        return;
    };
    if !dts_expansion_trace_enabled() && !super::counters_enabled() {
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    let unique_set = trace.overload_declaration_sets.insert((
        key.containing_interface.clone(),
        key.ordered_members.clone(),
    ));
    let attempts = trace
        .overload_instantiations
        .entry(key.clone())
        .or_default();
    let unique_key = *attempts == 0;
    let duplicate = *attempts != 0;
    *attempts += 1;
    let totals = trace
        .interface_members
        .entry(key.containing_interface.clone())
        .or_default();
    totals.overload_constructions += 1;
    totals.unique_overload_keys += u64::from(unique_key);
    drop(trace);
    crate::program::record_program_counter(|c| {
        c.unique_interface_overload_declaration_set_count += u64::from(unique_set);
        c.unique_interface_overload_instantiation_count += u64::from(unique_key);
        c.duplicate_interface_overload_construction_count += u64::from(duplicate);
        c.interface_overload_array_avoided_count += u64::from(cache_hit);
    });
}

pub(crate) fn record_interface_member_declaration_visits(count: usize) {
    crate::program::record_program_counter(|c| {
        c.interface_member_declaration_visit_count += count as u64
    });
}

#[derive(Default)]
struct ExpansionTotals {
    structural_expansions: u64,
    object_types: u64,
    function_types: u64,
    union_types: u64,
}

#[derive(Default)]
struct ExpansionTrace {
    high_water_bytes: u64,
    created_by_declaration: HashMap<String, u64>,
    peels_by_declaration: HashMap<String, u64>,
    peels_by_reason_and_declaration: HashMap<(DtsExpansionReason, String), u64>,
    generic_instantiations_by_declaration: HashMap<String, u64>,
    expansions_by_declaration: HashMap<String, ExpansionTotals>,
    expansions_by_file: HashMap<String, ExpansionTotals>,
    peel_reasons: HashMap<DtsExpansionReason, u64>,
    retained_nodes_by_module: HashMap<String, u64>,
    degraded_signature_attempts_by_declaration: HashMap<String, u64>,
    stable_interface_declarations: HashSet<StableInterfaceDeclarationId>,
    interface_instantiation_tuples: HashMap<InterfaceInstantiationKey, (u64, u64)>,
    interfaces: HashMap<StableInterfaceDeclarationId, InterfaceExpansionTotals>,
    inherited_merges: HashMap<String, u64>,
    member_declarations: HashSet<StableInterfaceMemberDeclarationId>,
    method_instantiations: HashMap<InterfaceMemberInstantiationKey, (u64, u64)>,
    overload_declaration_sets: HashSet<(
        StableInterfaceDeclarationId,
        Arc<[StableInterfaceMemberDeclarationId]>,
    )>,
    overload_instantiations: HashMap<InterfaceOverloadInstantiationKey, u64>,
    interface_members: HashMap<StableInterfaceDeclarationId, InterfaceMemberExpansionTotals>,
    interface_degradation_reasons:
        HashMap<(StableInterfaceDeclarationId, InterfaceDegradationReason), u64>,
    first_interface_degradation: HashMap<StableInterfaceDeclarationId, String>,
}

#[derive(Default)]
struct InterfaceMemberExpansionTotals {
    method_mapping_attempts: u64,
    unique_method_keys: u64,
    duplicate_clean_method_mappings: u64,
    duplicate_degraded_method_mappings: u64,
    overload_constructions: u64,
    unique_overload_keys: u64,
    inherited_method_units: u64,
}

#[derive(Default)]
struct InterfaceExpansionTotals {
    attempts: u64,
    clean_attempts: u64,
    degraded_attempts: u64,
    unique_argument_tuples: u64,
    duplicate_clean_attempts: u64,
    duplicate_degraded_attempts: u64,
    cache_hits: u64,
    function_types: u64,
    object_types: u64,
    types: u64,
    inherited_interfaces: usize,
    member_count: usize,
}

static TRACE: OnceLock<Mutex<ExpansionTrace>> = OnceLock::new();

pub(crate) fn dts_expansion_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_TRACE_DTS_EXPANSION").is_some())
}

fn dts_expansion_high_water_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_TRACE_DTS_HIGH_WATER").as_deref() != Ok("0"))
}

pub(crate) fn reset_dts_expansion_trace() {
    if let Some(trace) = TRACE.get()
        && let Ok(mut trace) = trace.lock()
    {
        *trace = ExpansionTrace::default();
    }
}

pub(crate) fn record_lazy_reference_created(key: &DeclarationResolutionKey) {
    crate::program::record_program_counter(|c| c.lazy_reference_create_count += 1);
    if !dts_expansion_trace_enabled() {
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    *trace
        .created_by_declaration
        .entry(declaration_label(key))
        .or_default() += 1;
}

pub(crate) fn record_generic_instantiation(key: &DeclarationResolutionKey) {
    crate::program::record_program_counter(|c| c.generic_instantiation_count += 1);
    if !dts_expansion_trace_enabled() {
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    *trace
        .generic_instantiations_by_declaration
        .entry(declaration_label(key))
        .or_default() += 1;
}

pub(crate) fn record_lazy_reference_peel_start(key: &DeclarationResolutionKey) {
    let reason = current_dts_expansion_reason();
    crate::program::record_program_counter(|c| {
        c.lazy_reference_peel_count += 1;
        if c.lazy_reference_peel_reason_counts.len() < 42 {
            c.lazy_reference_peel_reason_counts.resize(42, 0);
        }
        c.lazy_reference_peel_reason_counts[reason as usize] += 1;
    });
    if !dts_expansion_trace_enabled() {
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    *trace
        .peels_by_declaration
        .entry(declaration_label(key))
        .or_default() += 1;
    *trace
        .peels_by_reason_and_declaration
        .entry((reason, declaration_label(key)))
        .or_default() += 1;
    *trace.peel_reasons.entry(reason).or_default() += 1;
}

pub(crate) fn record_degraded_signature_expansion(key: &DeclarationResolutionKey) {
    if !super::counters_enabled() {
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    let attempts = trace
        .degraded_signature_attempts_by_declaration
        .entry(declaration_label(key))
        .or_default();
    *attempts += 1;
    let attempts = *attempts;
    crate::program::record_program_counter(|c| {
        if attempts == 1 {
            c.unique_degraded_signature_expansion_count += 1;
        } else {
            c.repeated_degraded_signature_expansion_count += 1;
            c.max_degraded_signature_expansion_repeats =
                c.max_degraded_signature_expansion_repeats.max(attempts);
        }
    });
}

pub(crate) fn record_lazy_reference_expansion(
    key: &DeclarationResolutionKey,
    module: &str,
    display: &str,
    resolution_depth: usize,
    before: TypeCreationSnapshot,
) {
    crate::program::record_program_counter(|c| c.lazy_reference_clean_expansion_count += 1);
    surge_ts_types::lazy_probe::count(if surge_ts_types::lazy_probe::in_relation() {
        "lazy_expansion.clean.in_relation"
    } else {
        "lazy_expansion.clean.outside"
    });
    let after = type_creation_snapshot();
    if !dts_expansion_trace_enabled() {
        return;
    }

    let objects = after.object_types.saturating_sub(before.object_types);
    let functions = after.function_types.saturating_sub(before.function_types);
    let unions = after.union_types.saturating_sub(before.union_types);
    let current_footprint = dts_expansion_high_water_enabled()
        .then(current_footprint_bytes)
        .flatten();
    let peak_footprint = dts_expansion_high_water_enabled()
        .then(peak_footprint_bytes)
        .flatten();
    let reason = current_dts_expansion_reason();
    let label = declaration_label(key);

    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    add_expansion(
        trace
            .expansions_by_declaration
            .entry(label.clone())
            .or_default(),
        objects,
        functions,
        unions,
    );
    add_expansion(
        trace
            .expansions_by_file
            .entry(key.file_name.as_ref().to_string())
            .or_default(),
        objects,
        functions,
        unions,
    );

    let high_water = peak_footprint.or(current_footprint).unwrap_or(0);
    if high_water > trace.high_water_bytes {
        trace.high_water_bytes = high_water;
        eprintln!(
            "{{\"dtsExpansionHighWater\":true,\"footprintBytes\":{},\"peakFootprintBytes\":{},\
             \"file\":{:?},\"module\":{:?},\"exportName\":{:?},\"operation\":{:?},\
             \"declarationKind\":\"named_type\",\"referenceId\":{:?},\
             \"resolutionDepth\":{},\"typesCreated\":{},\"objectTypesCreated\":{},\
             \"functionTypesCreated\":{},\"unionTypesCreated\":{}}}",
            current_footprint.map_or_else(|| "null".to_string(), |v| v.to_string()),
            peak_footprint.map_or_else(|| "null".to_string(), |v| v.to_string()),
            key.file_name,
            module,
            display,
            reason.label(),
            label,
            resolution_depth,
            objects + functions + unions,
            objects,
            functions,
            unions,
        );
    }
}

pub(crate) fn record_lazy_reference_expansion_start(
    key: &DeclarationResolutionKey,
    module: &str,
    display: &str,
    resolution_depth: usize,
) {
    if !dts_expansion_trace_enabled() {
        return;
    }
    if !dts_expansion_high_water_enabled() {
        return;
    }
    let current = current_footprint_bytes();
    let peak = peak_footprint_bytes();
    let high_water = peak.or(current).unwrap_or(0);
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    if high_water <= trace.high_water_bytes {
        return;
    }
    trace.high_water_bytes = high_water;
    eprintln!(
        "{{\"dtsExpansionHighWater\":true,\"footprintBytes\":{},\"peakFootprintBytes\":{},\
         \"file\":{:?},\"module\":{:?},\"exportName\":{:?},\
         \"operation\":{:?},\"declarationKind\":\"named_type\",\
         \"referenceId\":{:?},\"resolutionDepth\":{},\"typesCreated\":0,\
         \"objectTypesCreated\":0,\"functionTypesCreated\":0,\
         \"unionTypesCreated\":0}}",
        current.map_or_else(|| "null".to_string(), |v| v.to_string()),
        peak.map_or_else(|| "null".to_string(), |v| v.to_string()),
        key.file_name,
        module,
        display,
        current_dts_expansion_reason().label(),
        declaration_label(key),
        resolution_depth,
    );
}

pub(crate) fn record_retained_export_nodes(module: &str, nodes: u64) {
    if !dts_expansion_trace_enabled() {
        return;
    }
    let mut trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    trace
        .retained_nodes_by_module
        .insert(module.to_string(), nodes);
}

pub(crate) fn render_dts_expansion_summary() {
    if !dts_expansion_trace_enabled() {
        return;
    }
    let trace = TRACE
        .get_or_init(|| Mutex::new(ExpansionTrace::default()))
        .lock()
        .expect("d.ts expansion trace poisoned");
    eprintln!("d.ts expansion summary:");
    render_expansion_top(
        "top files by object types created",
        &trace.expansions_by_file,
        |v| v.object_types,
    );
    render_expansion_top(
        "top declarations by structural expansion count",
        &trace.expansions_by_declaration,
        |v| v.structural_expansions,
    );
    render_expansion_top(
        "top declarations by function types created",
        &trace.expansions_by_declaration,
        |v| v.function_types,
    );
    render_expansion_top_filtered(
        "top physical default-lib declarations by function types created",
        &trace.expansions_by_declaration,
        |name| {
            declaration_file(name)
                .is_some_and(crate::default_lib::is_physical_default_lib_file_name)
        },
        |v| v.function_types,
    );
    render_expansion_top_filtered(
        "top external dependency declarations by function types created",
        &trace.expansions_by_declaration,
        |name| {
            declaration_file(name).is_some_and(|file| {
                file.contains("node_modules")
                    && !crate::default_lib::is_physical_default_lib_file_name(file)
            })
        },
        |v| v.function_types,
    );
    render_count_top(
        "top lazy references by peel count",
        &trace.peels_by_declaration,
    );
    render_reason_count_top(
        "top lazy references peeled during module export collection",
        &trace.peels_by_reason_and_declaration,
        DtsExpansionReason::ModuleExportCollection,
    );
    render_count_top(
        "top generic aliases by instantiation count",
        &trace.generic_instantiations_by_declaration,
    );
    render_count_top(
        "top modules by retained export-table type nodes",
        &trace.retained_nodes_by_module,
    );
    render_count_top(
        "top degraded signature annotations by expansion attempts",
        &trace.degraded_signature_attempts_by_declaration,
    );
    render_interface_top(
        "top repeated interface instantiations",
        &trace.interfaces,
        |_| true,
    );
    render_interface_top(
        "top repeated physical default-lib interface instantiations",
        &trace.interfaces,
        |declaration| {
            crate::default_lib::is_physical_default_lib_file_name(&declaration.canonical_file)
        },
    );
    render_count_top(
        "top inherited member merge sources",
        &trace.inherited_merges,
    );
    render_count_top_filtered(
        "top physical default-lib inherited member merge sources",
        &trace.inherited_merges,
        |label| {
            declaration_file(label)
                .is_some_and(crate::default_lib::is_physical_default_lib_file_name)
        },
    );
    render_interface_member_top(&trace.interface_members);
    render_interface_degradation_top(
        &trace.interface_degradation_reasons,
        &trace.first_interface_degradation,
    );
    let clean_attempts = trace
        .interfaces
        .values()
        .map(|totals| totals.clean_attempts)
        .sum::<u64>();
    let unique_tuples = trace.interface_instantiation_tuples.len() as u64;
    eprintln!(
        "  clean interface resolution ratio: {clean_attempts}/{unique_tuples} = {:.2}",
        if unique_tuples == 0 {
            0.0
        } else {
            clean_attempts as f64 / unique_tuples as f64
        }
    );
    let physical_clean_attempts = trace
        .interfaces
        .iter()
        .filter(|(declaration, _)| {
            crate::default_lib::is_physical_default_lib_file_name(&declaration.canonical_file)
        })
        .map(|(_, totals)| totals.clean_attempts)
        .sum::<u64>();
    let physical_unique_tuples = trace
        .interface_instantiation_tuples
        .keys()
        .filter(|key| {
            crate::default_lib::is_physical_default_lib_file_name(&key.declaration.canonical_file)
        })
        .count() as u64;
    eprintln!(
        "  physical default-lib clean interface resolution ratio: {physical_clean_attempts}/{physical_unique_tuples} = {:.2}",
        if physical_unique_tuples == 0 {
            0.0
        } else {
            physical_clean_attempts as f64 / physical_unique_tuples as f64
        }
    );
    let mut reasons: Vec<_> = trace.peel_reasons.iter().collect();
    reasons.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    eprintln!("  lazy-reference peel reasons:");
    for (reason, count) in reasons {
        eprintln!("    {}: {count}", reason.label());
    }
}

fn render_interface_top(
    title: &str,
    values: &HashMap<StableInterfaceDeclarationId, InterfaceExpansionTotals>,
    include: impl Fn(&StableInterfaceDeclarationId) -> bool,
) {
    let mut rows: Vec<_> = values
        .iter()
        .filter(|(declaration, _)| include(declaration))
        .collect();
    rows.sort_by_key(|(_, totals)| std::cmp::Reverse(totals.attempts));
    eprintln!("  {title}:");
    for (declaration, totals) in rows.into_iter().take(20) {
        eprintln!(
            "    attempts={} unique_args={} cache_hits={} clean_duplicates={} degraded_duplicates={} clean={} degraded={} functions={} objects={} inherited={} members={} {}::{}",
            totals.attempts,
            totals.unique_argument_tuples,
            totals.cache_hits,
            totals.duplicate_clean_attempts,
            totals.duplicate_degraded_attempts,
            totals.clean_attempts,
            totals.degraded_attempts,
            totals.function_types,
            totals.object_types,
            totals.inherited_interfaces,
            totals.member_count,
            declaration.canonical_file,
            declaration.declaration_name,
        );
    }
}

fn render_interface_member_top(
    values: &HashMap<StableInterfaceDeclarationId, InterfaceMemberExpansionTotals>,
) {
    let mut rows: Vec<_> = values
        .iter()
        .filter(|(declaration, _)| {
            crate::default_lib::is_physical_default_lib_file_name(&declaration.canonical_file)
        })
        .collect();
    rows.sort_by_key(|(_, totals)| std::cmp::Reverse(totals.method_mapping_attempts));
    eprintln!("  top physical default-lib interface member mappings:");
    for (declaration, totals) in rows.into_iter().take(20) {
        eprintln!(
            "    methods={} unique_method_keys={} duplicate_clean={} duplicate_degraded={} overloads={} unique_overload_keys={} inherited_method_units={} {}::{}",
            totals.method_mapping_attempts,
            totals.unique_method_keys,
            totals.duplicate_clean_method_mappings,
            totals.duplicate_degraded_method_mappings,
            totals.overload_constructions,
            totals.unique_overload_keys,
            totals.inherited_method_units,
            declaration.canonical_file,
            declaration.declaration_name,
        );
    }
}

fn render_interface_degradation_top(
    values: &HashMap<(StableInterfaceDeclarationId, InterfaceDegradationReason), u64>,
    first: &HashMap<StableInterfaceDeclarationId, String>,
) {
    let mut interfaces = values
        .keys()
        .map(|(declaration, _)| declaration)
        .filter(|declaration| {
            crate::default_lib::is_physical_default_lib_file_name(&declaration.canonical_file)
        })
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    interfaces.sort_by_key(|declaration| {
        std::cmp::Reverse(
            values
                .iter()
                .filter(|((current, _), _)| current == declaration)
                .map(|(_, count)| *count)
                .sum::<u64>(),
        )
    });
    eprintln!("  top physical default-lib interface member degradation reasons:");
    for declaration in interfaces.into_iter().take(20) {
        let mut reasons = values
            .iter()
            .filter(|((current, _), _)| current == &declaration)
            .map(|((_, reason), count)| (*reason, *count))
            .collect::<Vec<_>>();
        reasons.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        let reasons = reasons
            .into_iter()
            .map(|(reason, count)| format!("{}={count}", reason.label()))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!(
            "    {}::{} first={} {}",
            declaration.canonical_file,
            declaration.declaration_name,
            first
                .get(&declaration)
                .map_or("none", std::string::String::as_str),
            reasons,
        );
    }
}

fn declaration_label(key: &DeclarationResolutionKey) -> String {
    format!("{}::{}", key.file_name, key.name)
}

fn declaration_file(label: &str) -> Option<&str> {
    label.rsplit_once("::").map(|(file, _)| file)
}

fn add_expansion(totals: &mut ExpansionTotals, objects: u64, functions: u64, unions: u64) {
    totals.structural_expansions += 1;
    totals.object_types += objects;
    totals.function_types += functions;
    totals.union_types += unions;
}

fn render_expansion_top(
    title: &str,
    values: &HashMap<String, ExpansionTotals>,
    value: impl Fn(&ExpansionTotals) -> u64,
) {
    let mut rows: Vec<_> = values
        .iter()
        .map(|(name, totals)| (name, value(totals)))
        .collect();
    rows.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    eprintln!("  {title}:");
    for (name, count) in rows.into_iter().take(10) {
        eprintln!("    {count}: {name}");
    }
}

fn render_expansion_top_filtered(
    title: &str,
    values: &HashMap<String, ExpansionTotals>,
    include: impl Fn(&str) -> bool,
    value: impl Fn(&ExpansionTotals) -> u64,
) {
    let mut rows: Vec<_> = values
        .iter()
        .filter(|(name, _)| include(name))
        .map(|(name, totals)| (name, value(totals)))
        .collect();
    rows.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    eprintln!("  {title}:");
    for (name, count) in rows.into_iter().take(10) {
        eprintln!("    {count}: {name}");
    }
}

fn render_count_top(title: &str, values: &HashMap<String, u64>) {
    let mut rows: Vec<_> = values.iter().collect();
    rows.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    eprintln!("  {title}:");
    for (name, count) in rows.into_iter().take(10) {
        eprintln!("    {count}: {name}");
    }
}

fn render_count_top_filtered(
    title: &str,
    values: &HashMap<String, u64>,
    include: impl Fn(&str) -> bool,
) {
    let mut rows: Vec<_> = values.iter().filter(|(label, _)| include(label)).collect();
    rows.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    eprintln!("  {title}:");
    for (name, count) in rows.into_iter().take(10) {
        eprintln!("    {count}: {name}");
    }
}

fn render_reason_count_top(
    title: &str,
    values: &HashMap<(DtsExpansionReason, String), u64>,
    reason: DtsExpansionReason,
) {
    let mut rows: Vec<_> = values
        .iter()
        .filter_map(|((entry_reason, name), count)| {
            (*entry_reason == reason).then_some((name, count))
        })
        .collect();
    rows.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    eprintln!("  {title}:");
    for (name, count) in rows.into_iter().take(10) {
        eprintln!("    {count}: {name}");
    }
}
