//! Full retained-heap census (`SURGE_RETENTION_CENSUS=1`).
//!
//! Unlike the cache-focused `type_graph_census`, this walks every structure the
//! program actually retains at a stage boundary — module analyses (symbol
//! tables, export tables, declaration tables), import bindings, resolution
//! scopes, globals, the checker caches, and the canonical type store — and
//! attributes estimated live bytes to named owner groups. All shared payloads
//! (`Arc`s, declaration tables, property maps) are pointer-deduplicated so a payload
//! reachable from several owners is charged once, to the first group that
//! reaches it. Estimates are shallow-struct plus owned-heap models, not
//! allocator ground truth; compare group deltas, not absolute totals.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::mem::size_of;
use std::sync::Arc;

use surge_ts_syntax::{
    ParsedFunctionType, ParsedInterfaceMember, ParsedNamedType, ParsedType, ParsedTypeParameter,
};
use surge_ts_types::fx::FxHashMap;
use surge_ts_types::{FunctionType, FunctionTypePayload, ObjectType, Type, TypeReference};

use crate::context::{CheckerContext, DeclarationResolutionState};
use crate::modules::{ModuleExportTable, ModuleImportBindings};
use crate::symbols::{
    SymbolInfo, SymbolTable, TypeDeclarationInfo, TypeDeclarationScope, TypeDeclarationTable,
};

pub(crate) fn retention_census_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_RETENTION_CENSUS").is_some())
}

/// Everything the program pipeline retains that the census should attribute.
/// All fields are optional so call sites can report whatever exists at their
/// stage.
#[derive(Default)]
pub(crate) struct RetentionCensusView<'a> {
    pub(crate) module_analyses: Option<&'a [Option<crate::program::ModuleAnalysis>]>,
    pub(crate) preliminary_module_analyses: Option<&'a [Option<crate::program::ModuleAnalysis>]>,
    pub(crate) module_import_bindings: Option<&'a [Option<ModuleImportBindings>]>,
    pub(crate) preliminary_module_import_bindings: Option<&'a [Option<ModuleImportBindings>]>,
    pub(crate) module_resolution_scopes: Option<&'a [Option<Arc<TypeDeclarationScope>>]>,
    pub(crate) parsed_files: Option<&'a [crate::program::ParsedProgramFile]>,
    pub(crate) global_symbols: Option<&'a SymbolTable>,
    pub(crate) function_signatures: Option<&'a [&'a surge_ts_types::FunctionType]>,
}

#[derive(Default, Debug, Clone, Copy)]
struct GroupTally {
    bytes: u64,
    items: u64,
}

#[derive(Default)]
struct FallbackClassCounts {
    unknown_containing: u64,
    context_retaining_reference: u64,
    over_budget: u64,
    internable: u64,
}

struct Walker {
    // Pointer-dedup sets. usize keys are payload addresses.
    seen_symbol_infos: FxHashMap<usize, ()>,
    seen_function_payloads: FxHashMap<usize, ()>,
    seen_parameter_lists: FxHashMap<usize, ()>,
    seen_property_maps: FxHashMap<usize, ()>,
    seen_union_payloads: FxHashMap<usize, ()>,
    seen_resolvers: FxHashMap<usize, ()>,
    seen_decl_tables: FxHashMap<usize, ()>,
    seen_decl_payloads: FxHashMap<usize, ()>,
    seen_decl_bodies: FxHashMap<usize, ()>,
    seen_symbol_maps: FxHashMap<usize, ()>,
    seen_span_maps: FxHashMap<usize, ()>,
    seen_scopes: FxHashMap<usize, ()>,
    seen_shared_captures: FxHashMap<usize, ()>,
    seen_resolved_memos: FxHashMap<usize, ()>,
    seen_signature_infos: FxHashMap<usize, ()>,
    seen_header_strings: FxHashMap<usize, ()>,
    groups: HashMap<&'static str, GroupTally>,
    sub_tallies: HashMap<&'static str, HashMap<&'static str, u64>>,
    current_group: &'static str,
    // Global tallies independent of groups.
    canonical_function_payloads: u64,
    fallback_function_payloads: u64,
    fallback_function_bytes: u64,
    fallback_classes: FallbackClassCounts,
    reference_count: u64,
    reference_argument_slots: u64,
    resolver_count: u64,
    resolver_own_bytes: u64,
    resolver_shared_bytes: u64,
    environment_count: u64,
    environment_index_bytes: u64,
    declaration_table_instances: u64,
    declaration_index_bytes: u64,
    symbol_count: u64,
    parsed_signature_bytes: u64,
    declaration_entries: u64,
    declaration_parsed_bytes: u64,
    span_map_entries: u64,
    span_map_bytes: u64,
    scope_self_cycle_declarations: u64,
    scope_attached_declarations: u64,
}

impl Walker {
    fn new() -> Self {
        Self {
            seen_symbol_infos: FxHashMap::default(),
            seen_function_payloads: FxHashMap::default(),
            seen_parameter_lists: FxHashMap::default(),
            seen_property_maps: FxHashMap::default(),
            seen_union_payloads: FxHashMap::default(),
            seen_resolvers: FxHashMap::default(),
            seen_decl_tables: FxHashMap::default(),
            seen_decl_payloads: FxHashMap::default(),
            seen_decl_bodies: FxHashMap::default(),
            seen_symbol_maps: FxHashMap::default(),
            seen_span_maps: FxHashMap::default(),
            seen_scopes: FxHashMap::default(),
            seen_shared_captures: FxHashMap::default(),
            seen_resolved_memos: FxHashMap::default(),
            seen_signature_infos: FxHashMap::default(),
            seen_header_strings: FxHashMap::default(),
            groups: HashMap::new(),
            sub_tallies: HashMap::new(),
            current_group: "unattributed",
            canonical_function_payloads: 0,
            fallback_function_payloads: 0,
            fallback_function_bytes: 0,
            fallback_classes: FallbackClassCounts::default(),
            reference_count: 0,
            reference_argument_slots: 0,
            resolver_count: 0,
            resolver_own_bytes: 0,
            resolver_shared_bytes: 0,
            environment_count: 0,
            environment_index_bytes: 0,
            declaration_table_instances: 0,
            declaration_index_bytes: 0,
            symbol_count: 0,
            parsed_signature_bytes: 0,
            declaration_entries: 0,
            declaration_parsed_bytes: 0,
            span_map_entries: 0,
            span_map_bytes: 0,
            scope_self_cycle_declarations: 0,
            scope_attached_declarations: 0,
        }
    }

    fn add(&mut self, bytes: u64) {
        let tally = self.groups.entry(self.current_group).or_default();
        tally.bytes += bytes;
    }

    fn add_item(&mut self) {
        let tally = self.groups.entry(self.current_group).or_default();
        tally.items += 1;
    }

    fn sub(&mut self, key: &'static str, bytes: u64) {
        *self
            .sub_tallies
            .entry(self.current_group)
            .or_default()
            .entry(key)
            .or_default() += bytes;
    }

    /// Charges an `Arc<str>` header string once per distinct allocation.
    fn header_string_bytes(&mut self, value: &Arc<str>) -> u64 {
        let address = value.as_ptr() as usize;
        if Walker::first_visit(&mut self.seen_header_strings, address) {
            value.len() as u64
        } else {
            0
        }
    }

    fn first_visit(map: &mut FxHashMap<usize, ()>, address: usize) -> bool {
        match map.entry(address) {
            Entry::Vacant(entry) => {
                entry.insert(());
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    fn walk_symbol_table(&mut self, table: &SymbolTable) {
        let map_address = table.symbols_map_address();
        if Walker::first_visit(&mut self.seen_symbol_maps, map_address) {
            for (name, symbol) in table.iter_shared() {
                let name_bytes =
                    (size_of::<Arc<str>>() + name.len() + size_of::<usize>() * 2) as u64;
                self.add(name_bytes);
                self.sub("symbol_names", name_bytes);
                self.walk_symbol_info(symbol);
            }
        }
        let span_map_address = table.declaration_spans_map_address();
        if Walker::first_visit(&mut self.seen_span_maps, span_map_address) {
            let (entries, bytes) = table.declaration_spans_footprint();
            self.span_map_entries += entries;
            self.span_map_bytes += bytes;
            self.add(bytes);
        }
        if let Some(parent) = table.parent_table() {
            self.walk_symbol_table(parent);
        }
    }

    fn walk_symbol_info(&mut self, symbol: &Arc<SymbolInfo>) {
        if !Walker::first_visit(&mut self.seen_symbol_infos, Arc::as_ptr(symbol) as usize) {
            return;
        }
        self.symbol_count += 1;
        self.add_item();
        self.add(size_of::<SymbolInfo>() as u64);
        self.sub("symbol_struct", size_of::<SymbolInfo>() as u64);
        if let Some(signature) = symbol.function_signature.as_ref()
            && Walker::first_visit(
                &mut self.seen_signature_infos,
                Arc::as_ptr(signature) as usize,
            )
        {
            let mut tp_bytes = 0u64;
            let mut tp_unique = 0u64;
            for parameter in &signature.type_parameters {
                tp_bytes += parsed_type_parameter_bytes(parameter);
                tp_unique += parsed_type_parameter_unique_bytes(parameter);
            }
            let mut pt_bytes = 0u64;
            let mut pt_unique = 0u64;
            for parameter in &signature.parameter_types {
                pt_bytes += size_of::<Option<ParsedType>>() as u64;
                pt_unique += size_of::<Option<ParsedType>>() as u64;
                if let Some(ty) = parameter {
                    pt_bytes += parsed_type_bytes(ty);
                    pt_unique += parsed_type_unique_bytes(ty);
                }
            }
            let mut rt_bytes = 0u64;
            let mut rt_unique = 0u64;
            if let Some(return_type) = signature.return_type.as_ref() {
                rt_bytes += parsed_type_bytes(return_type);
                rt_unique += parsed_type_unique_bytes(return_type);
            }
            let df_bytes = signature
                .declaring_file
                .as_ref()
                .map_or(0, |file| file.len() as u64 + size_of::<Arc<str>>() as u64);
            let pn_bytes: u64 = signature
                .parameter_names
                .iter()
                .map(|name| {
                    size_of::<Option<String>>() as u64
                        + name.as_ref().map_or(0, |n| n.capacity() as u64)
                })
                .sum();
            self.sub("sig_count", 1);
            self.sub("sig_type_parameters", tp_bytes);
            self.sub("sig_parameter_types", pt_bytes);
            self.sub("sig_return_type", rt_bytes);
            self.sub("sig_declaring_file", df_bytes);
            self.sub("sig_parameter_names_UNCENSUSED", pn_bytes);
            self.sub(
                "sig_unique_nonarc",
                tp_unique + pt_unique + rt_unique + df_bytes + pn_bytes,
            );
            let bytes = tp_bytes
                + pt_bytes
                + rt_bytes
                + df_bytes
                + size_of::<crate::symbols::FunctionSignatureInfo>() as u64;
            self.parsed_signature_bytes += bytes;
            self.add(bytes);
        }
        self.walk_type(&symbol.ty);
    }

    fn walk_type(&mut self, ty: &Type) {
        match ty {
            Type::Function(function) => self.walk_function(function),
            Type::Object(object) => self.walk_object(object),
            Type::Array(element) => {
                self.add(size_of::<Type>() as u64);
                self.walk_type(element);
            }
            Type::Tuple(elements) => {
                self.add((elements.len() * size_of::<Type>()) as u64);
                for element in elements {
                    self.walk_type(element);
                }
            }
            Type::Union(union) => {
                if Walker::first_visit(&mut self.seen_union_payloads, union.payload_address()) {
                    let union_bytes = (size_of::<surge_ts_types::UnionTypePayload>()
                        + union.types().len() * size_of::<Type>())
                        as u64;
                    self.add(union_bytes);
                    self.sub("union_payloads", union_bytes);
                    for member in union.types() {
                        self.walk_type(member);
                    }
                }
            }
            Type::Reference(reference) => self.walk_reference(reference),
            Type::StringLiteral(value) => self.add(value.capacity() as u64),
            Type::NumberLiteral(value) => self.add(value.value.capacity() as u64),
            _ => {}
        }
    }

    fn walk_function(&mut self, function: &FunctionType) {
        let payload_address = function.payload_address();
        if !Walker::first_visit(&mut self.seen_function_payloads, payload_address) {
            return;
        }
        let is_fallback = function.id().is_none();
        let payload_bytes = size_of::<FunctionTypePayload>() as u64;
        self.add(payload_bytes);
        self.sub("function_payloads", payload_bytes);
        if is_fallback {
            self.fallback_function_payloads += 1;
        } else {
            self.canonical_function_payloads += 1;
        }
        let parameters = function.parameters();
        let list_bytes = (parameters.len() * size_of::<Type>()) as u64;
        if Walker::first_visit(
            &mut self.seen_parameter_lists,
            function.parameter_list_address(),
        ) {
            self.add(list_bytes);
            self.sub("function_param_lists", list_bytes);
            for parameter in parameters {
                self.walk_type(parameter);
            }
        }
        if is_fallback {
            self.fallback_function_bytes += payload_bytes + list_bytes;
            match classify_fallback(function) {
                FallbackClass::UnknownContaining => {
                    self.fallback_classes.unknown_containing += 1;
                }
                FallbackClass::ContextRetainingReference => {
                    self.fallback_classes.context_retaining_reference += 1;
                }
                FallbackClass::OverBudget => self.fallback_classes.over_budget += 1,
                FallbackClass::Internable => self.fallback_classes.internable += 1,
            }
        }
        self.walk_type(function.return_type());
    }

    fn walk_object(&mut self, object: &ObjectType) {
        self.add(size_of::<ObjectType>() as u64);
        self.sub("object_structs", size_of::<ObjectType>() as u64);
        if Walker::first_visit(
            &mut self.seen_property_maps,
            Arc::as_ptr(&object.properties) as usize,
        ) {
            for (name, property) in object.properties.iter() {
                let prop_bytes = (size_of::<Arc<str>>() + name.len()) as u64
                    + size_of::<surge_ts_types::ObjectProperty>() as u64;
                self.add(prop_bytes);
                self.sub("object_props", prop_bytes);
                self.walk_type(&property.ty);
            }
        }
        if let Some(index) = object.string_index_type.as_deref() {
            self.walk_type(index);
        }
        if let Some(call) = object.call_signature() {
            self.walk_function(call);
        }
        if let Some(construct) = object.construct_signature() {
            self.walk_function(construct);
        }
    }

    fn walk_reference(&mut self, reference: &TypeReference) {
        self.reference_count += 1;
        self.reference_argument_slots += reference.arguments.len() as u64;
        let ref_bytes = (size_of::<TypeReference>()
            + reference.arguments.len() * size_of::<Type>()
            + reference.id.len()
            + reference.display.len()) as u64;
        self.add(ref_bytes);
        self.sub("type_reference_own", ref_bytes);
        for argument in reference.arguments.iter() {
            self.walk_type(argument);
        }
        if Walker::first_visit(&mut self.seen_resolvers, reference.resolver_address()) {
            self.resolver_count += 1;
            let census = reference.captured_census();
            self.resolver_own_bytes += census.own_bytes;
            self.add(census.own_bytes);
            self.sub("resolver_own", census.own_bytes);
            for (address, bytes) in census.shared_captures {
                if Walker::first_visit(&mut self.seen_shared_captures, address) {
                    self.resolver_shared_bytes += bytes;
                    self.add(bytes);
                    self.sub("resolver_shared", bytes);
                }
            }
            if let Some(resolved) = reference.peek_resolved()
                && Walker::first_visit(
                    &mut self.seen_resolved_memos,
                    Arc::as_ptr(&resolved) as usize,
                )
            {
                self.walk_type(&resolved);
            }
        }
    }

    fn walk_declaration_table(&mut self, table: &TypeDeclarationTable) {
        let address = table.identity_address();
        if !Walker::first_visit(&mut self.seen_decl_tables, address) {
            return;
        }
        self.declaration_table_instances += 1;
        self.declaration_index_bytes += table.index_heap_bytes();
        self.add(table.index_heap_bytes());
        self.sub("decl_index", table.index_heap_bytes());
        for (_, declaration) in table.iter() {
            // Cycle probe: a payload whose attached resolution scope carries
            // the payload's own table as a layer forms a strong
            // scope->table->payload->scope cycle, which decides whether the
            // per-file scope-release lever can free anything.
            let scope = match declaration {
                TypeDeclarationInfo::Alias(info) => info.resolution_scope.as_ref(),
                TypeDeclarationInfo::Interface(info) => info.resolution_scope.as_ref(),
            };
            if let Some(scope) = scope {
                self.scope_attached_declarations += 1;
                if scope
                    .census_layer_addresses()
                    .any(|layer_address| layer_address == address)
                {
                    self.scope_self_cycle_declarations += 1;
                }
            }
            self.walk_declaration_info(declaration);
        }
    }

    fn walk_declaration_info(&mut self, declaration: &TypeDeclarationInfo) {
        // Table clones share declaration payloads by `Arc`; charge each payload
        // once no matter how many table instances index it.
        if !Walker::first_visit(
            &mut self.seen_decl_payloads,
            declaration as *const TypeDeclarationInfo as usize,
        ) {
            return;
        }
        self.declaration_entries += 1;
        match declaration {
            TypeDeclarationInfo::Alias(info) => {
                let meta_bytes = self.header_string_bytes(&info.name)
                    + self.header_string_bytes(&info.file_name)
                    + info
                        .declared_name
                        .as_ref()
                        .map_or(0, |declared_name| self.header_string_bytes(declared_name))
                    + size_of::<TypeDeclarationInfo>() as u64;
                self.add(meta_bytes);
                self.sub("decl_meta", meta_bytes);
                if let Some(scope) = info.resolution_scope.as_ref() {
                    self.walk_scope(scope);
                }
                if Walker::first_visit(&mut self.seen_decl_bodies, Arc::as_ptr(&info.body) as usize)
                {
                    let mut bytes = 0u64;
                    for parameter in &info.body.type_parameters {
                        bytes += parsed_type_parameter_bytes(parameter);
                    }
                    bytes += parsed_type_bytes(&info.body.ty);
                    self.declaration_parsed_bytes += bytes;
                    self.add(bytes);
                    self.sub("decl_parsed", bytes);
                }
            }
            TypeDeclarationInfo::Interface(info) => {
                let meta_bytes = self.header_string_bytes(&info.name)
                    + self.header_string_bytes(&info.file_name)
                    + info
                        .declared_name
                        .as_ref()
                        .map_or(0, |declared_name| self.header_string_bytes(declared_name))
                    + size_of::<TypeDeclarationInfo>() as u64;
                self.add(meta_bytes);
                self.sub("decl_meta", meta_bytes);
                if let Some(scope) = info.resolution_scope.as_ref() {
                    self.walk_scope(scope);
                }
                if Walker::first_visit(&mut self.seen_decl_bodies, Arc::as_ptr(&info.body) as usize)
                {
                    let mut bytes = 0u64;
                    for parameter in &info.body.type_parameters {
                        bytes += parsed_type_parameter_bytes(parameter);
                    }
                    for extend in &info.body.extends {
                        bytes += parsed_named_type_bytes(extend);
                    }
                    for member in &info.body.members {
                        bytes += parsed_interface_member_bytes(member);
                    }
                    if let Some(index) = info.body.string_index_type.as_ref() {
                        bytes += parsed_type_bytes(index);
                    }
                    for call in info.body.call_signature.iter() {
                        bytes += parsed_function_type_bytes(call);
                    }
                    for construct in &info.body.construct_signatures {
                        bytes += parsed_function_type_bytes(construct);
                    }
                    let fragment_count =
                        info.body.declaration_fragments.len() + info.body.member_fragments.len();
                    bytes += (fragment_count * size_of::<usize>() * 5) as u64;
                    self.declaration_parsed_bytes += bytes;
                    self.add(bytes);
                    self.sub("decl_parsed", bytes);
                }
            }
        }
    }

    fn walk_scope(&mut self, scope: &Arc<TypeDeclarationScope>) {
        if !Walker::first_visit(&mut self.seen_scopes, Arc::as_ptr(scope) as usize) {
            return;
        }
        for layer in scope.layers() {
            self.walk_declaration_table(layer);
        }
    }

    fn walk_export_table(&mut self, table: &ModuleExportTable) {
        self.walk_declaration_table(&table.type_declarations);
        self.walk_symbol_table(&table.symbols);
        if let Some(symbol) = table.default_symbol.as_ref() {
            self.walk_symbol_info(symbol);
        }
        if let Some(symbol) = table.export_assignment_symbol.as_ref() {
            self.walk_symbol_info(symbol);
        }
        if let Some(ty) = table.namespace_export_object_type.as_ref() {
            self.walk_type(ty);
        }
    }
}

enum FallbackClass {
    UnknownContaining,
    ContextRetainingReference,
    OverBudget,
    Internable,
}

fn classify_fallback(function: &FunctionType) -> FallbackClass {
    let mut nodes = 0usize;
    let mut found_unknown = false;
    let mut found_context_reference = false;
    let mut over_budget = false;
    fn visit(
        ty: &Type,
        depth: usize,
        nodes: &mut usize,
        found_unknown: &mut bool,
        found_context_reference: &mut bool,
        over_budget: &mut bool,
    ) {
        *nodes += 1;
        if *nodes > 128 || depth >= 16 {
            *over_budget = true;
            return;
        }
        match ty {
            Type::Unknown | Type::TypeParameter(_) => *found_unknown = true,
            Type::Function(function) => {
                for parameter in function.parameters() {
                    visit(
                        parameter,
                        depth + 1,
                        nodes,
                        found_unknown,
                        found_context_reference,
                        over_budget,
                    );
                }
                visit(
                    function.return_type(),
                    depth + 1,
                    nodes,
                    found_unknown,
                    found_context_reference,
                    over_budget,
                );
            }
            Type::Object(object) => {
                for (_, property) in object.properties.iter() {
                    visit(
                        &property.ty,
                        depth + 1,
                        nodes,
                        found_unknown,
                        found_context_reference,
                        over_budget,
                    );
                }
            }
            Type::Array(element) => visit(
                element,
                depth + 1,
                nodes,
                found_unknown,
                found_context_reference,
                over_budget,
            ),
            Type::Tuple(elements) => {
                for element in elements {
                    visit(
                        element,
                        depth + 1,
                        nodes,
                        found_unknown,
                        found_context_reference,
                        over_budget,
                    );
                }
            }
            Type::Union(union) => {
                for member in union.types() {
                    visit(
                        member,
                        depth + 1,
                        nodes,
                        found_unknown,
                        found_context_reference,
                        over_budget,
                    );
                }
            }
            Type::Reference(reference) => {
                if reference.retains_resolution_context()
                    || !reference.supports_program_canonicalization()
                {
                    *found_context_reference = true;
                }
                for argument in reference.arguments.iter() {
                    visit(
                        argument,
                        depth + 1,
                        nodes,
                        found_unknown,
                        found_context_reference,
                        over_budget,
                    );
                }
            }
            _ => {}
        }
    }
    for parameter in function.parameters() {
        visit(
            parameter,
            0,
            &mut nodes,
            &mut found_unknown,
            &mut found_context_reference,
            &mut over_budget,
        );
    }
    visit(
        function.return_type(),
        0,
        &mut nodes,
        &mut found_unknown,
        &mut found_context_reference,
        &mut over_budget,
    );
    if found_unknown {
        FallbackClass::UnknownContaining
    } else if found_context_reference {
        FallbackClass::ContextRetainingReference
    } else if over_budget {
        FallbackClass::OverBudget
    } else {
        FallbackClass::Internable
    }
}

/// Like [`parsed_type_bytes`] but charges only heap that a `ParsedType::clone`
/// actually duplicates: `Arc`-backed variants (Named/Function/Union/…) share
/// their payload with the clone source, so they cost just the inline enum.
fn parsed_type_unique_bytes(ty: &ParsedType) -> u64 {
    let own = size_of::<ParsedType>() as u64;
    own + match ty {
        ParsedType::StringLiteral(value) | ParsedType::NumberLiteral(value) => {
            value.capacity() as u64
        }
        ParsedType::Object(object) => {
            let mut bytes = 0u64;
            for property in &object.properties {
                bytes += property.name.capacity() as u64
                    + size_of::<surge_ts_syntax::ParsedObjectTypeProperty>() as u64
                    + parsed_type_unique_bytes(&property.ty);
            }
            if let Some(call) = object.call_signature.as_ref() {
                bytes += parsed_function_type_bytes(call);
            }
            bytes
        }
        ParsedType::Array(_)
        | ParsedType::KeyOf(_)
        | ParsedType::Tuple(..)
        | ParsedType::Union(_)
        | ParsedType::Intersection(_)
        | ParsedType::Function(_)
        | ParsedType::Named(_)
        | ParsedType::Predicate(_) => 0,
        ParsedType::TypeOf(type_of) => {
            type_of.name.capacity() as u64
                + type_of
                    .members
                    .iter()
                    .map(|member| member.capacity() as u64 + size_of::<String>() as u64)
                    .sum::<u64>()
        }
        ParsedType::IndexedAccess(indexed) => {
            parsed_type_unique_bytes(&indexed.object_type)
                + parsed_type_unique_bytes(&indexed.index_type)
        }
        ParsedType::Mapped(mapped) => {
            mapped.key_name.capacity() as u64
                + parsed_type_unique_bytes(&mapped.constraint)
                + parsed_type_unique_bytes(&mapped.value_type)
        }
        ParsedType::Conditional(conditional) => {
            parsed_type_unique_bytes(&conditional.check_type)
                + parsed_type_unique_bytes(&conditional.extends_type)
                + parsed_type_unique_bytes(&conditional.true_type)
                + parsed_type_unique_bytes(&conditional.false_type)
        }
        ParsedType::TemplateLiteral(template) => {
            template
                .quasis
                .iter()
                .map(|quasi| quasi.capacity() as u64 + size_of::<String>() as u64)
                .sum::<u64>()
                + template
                    .interpolations
                    .iter()
                    .map(parsed_type_unique_bytes)
                    .sum::<u64>()
        }
        ParsedType::Infer(infer) => infer.name.capacity() as u64,
        _ => 0,
    }
}

fn parsed_type_parameter_unique_bytes(parameter: &ParsedTypeParameter) -> u64 {
    let mut bytes = (size_of::<ParsedTypeParameter>() + parameter.name.capacity()) as u64;
    if let Some(constraint) = parameter.constraint.as_ref() {
        bytes += parsed_type_unique_bytes(constraint);
    }
    if let Some(default_type) = parameter.default_type.as_ref() {
        bytes += parsed_type_unique_bytes(default_type);
    }
    bytes
}

fn parsed_type_parameter_bytes(parameter: &ParsedTypeParameter) -> u64 {
    let mut bytes = (size_of::<ParsedTypeParameter>() + parameter.name.capacity()) as u64;
    if let Some(constraint) = parameter.constraint.as_ref() {
        bytes += parsed_type_bytes(constraint);
    }
    if let Some(default_type) = parameter.default_type.as_ref() {
        bytes += parsed_type_bytes(default_type);
    }
    bytes
}

fn parsed_named_type_bytes(named: &ParsedNamedType) -> u64 {
    let mut bytes = (size_of::<ParsedNamedType>() + named.name.capacity()) as u64;
    for argument in &named.type_arguments {
        bytes += parsed_type_bytes(argument);
    }
    bytes
}

fn parsed_interface_member_bytes(member: &ParsedInterfaceMember) -> u64 {
    (size_of::<ParsedInterfaceMember>() + member.name.capacity()) as u64
        + parsed_type_bytes(&member.ty)
}

fn parsed_function_type_bytes(function: &ParsedFunctionType) -> u64 {
    let mut bytes = size_of::<ParsedFunctionType>() as u64;
    for parameter in &function.parameters {
        bytes += size_of::<surge_ts_syntax::ParsedFunctionTypeParameter>() as u64;
        bytes += parameter
            .name
            .as_ref()
            .map_or(0, |name| name.capacity() as u64);
        bytes += parsed_type_bytes(&parameter.ty);
    }
    bytes += parsed_type_bytes(&function.return_type);
    for parameter in &function.type_parameters {
        bytes += parsed_type_parameter_bytes(parameter);
    }
    bytes
}

fn parsed_type_bytes(ty: &ParsedType) -> u64 {
    let own = size_of::<ParsedType>() as u64;
    own + match ty {
        ParsedType::StringLiteral(value) | ParsedType::NumberLiteral(value) => {
            value.capacity() as u64
        }
        ParsedType::Object(object) => {
            let mut bytes = 0u64;
            for property in &object.properties {
                bytes += property.name.capacity() as u64
                    + size_of::<surge_ts_syntax::ParsedObjectTypeProperty>() as u64
                    + parsed_type_bytes(&property.ty);
            }
            if let Some(call) = object.call_signature.as_ref() {
                bytes += parsed_function_type_bytes(call);
            }
            bytes
        }
        ParsedType::Array(element) | ParsedType::KeyOf(element) => parsed_type_bytes(element),
        ParsedType::Tuple(elements, ..)
        | ParsedType::Union(elements)
        | ParsedType::Intersection(elements) => elements.iter().map(parsed_type_bytes).sum::<u64>(),
        ParsedType::Function(function) => parsed_function_type_bytes(function),
        ParsedType::Named(named) => parsed_named_type_bytes(named),
        ParsedType::TypeOf(type_of) => {
            type_of.name.capacity() as u64
                + type_of
                    .members
                    .iter()
                    .map(|member| member.capacity() as u64 + size_of::<String>() as u64)
                    .sum::<u64>()
        }
        ParsedType::IndexedAccess(indexed) => {
            parsed_type_bytes(&indexed.object_type) + parsed_type_bytes(&indexed.index_type)
        }
        ParsedType::Mapped(mapped) => {
            mapped.key_name.capacity() as u64
                + parsed_type_bytes(&mapped.constraint)
                + parsed_type_bytes(&mapped.value_type)
        }
        ParsedType::Conditional(conditional) => {
            parsed_type_bytes(&conditional.check_type)
                + parsed_type_bytes(&conditional.extends_type)
                + parsed_type_bytes(&conditional.true_type)
                + parsed_type_bytes(&conditional.false_type)
        }
        ParsedType::TemplateLiteral(template) => {
            template
                .quasis
                .iter()
                .map(|quasi| quasi.capacity() as u64 + size_of::<String>() as u64)
                .sum::<u64>()
                + template
                    .interpolations
                    .iter()
                    .map(parsed_type_bytes)
                    .sum::<u64>()
        }
        ParsedType::Infer(infer) => infer.name.capacity() as u64,
        _ => 0,
    }
}

fn parsed_expression_bytes(expression: &surge_ts_syntax::ParsedExpression) -> u64 {
    use surge_ts_syntax::ParsedExpression as E;
    let own = size_of::<E>() as u64;
    own + match expression {
        E::StringLiteral(value) | E::NumberLiteral(value) => value.capacity() as u64,
        E::Identifier { name, .. } => name.capacity() as u64,
        E::ObjectLiteral { properties, .. } => properties
            .iter()
            .map(|property| {
                size_of::<surge_ts_syntax::ParsedObjectProperty>() as u64
                    + property.name.capacity() as u64
                    + parsed_expression_bytes(&property.value)
            })
            .sum(),
        E::ArrayLiteral { elements, .. } => elements
            .iter()
            .map(|element| {
                size_of::<surge_ts_syntax::ParsedArrayElement>() as u64
                    + parsed_expression_bytes(&element.expression)
            })
            .sum(),
        E::TemplateLiteral { expressions, .. } => {
            expressions.iter().map(parsed_expression_bytes).sum()
        }
        E::Unary { operand, .. } => parsed_expression_bytes(operand),
        E::Binary { left, right, .. }
        | E::Logical { left, right, .. }
        | E::NullishCoalescing { left, right, .. } => {
            parsed_expression_bytes(left) + parsed_expression_bytes(right)
        }
        E::Conditional {
            condition,
            when_true,
            when_false,
            ..
        } => {
            parsed_expression_bytes(condition)
                + parsed_expression_bytes(when_true)
                + parsed_expression_bytes(when_false)
        }
        E::PropertyAccess {
            object,
            property_name,
            ..
        }
        | E::OptionalPropertyAccess {
            object,
            property_name,
            ..
        } => property_name.capacity() as u64 + parsed_expression_bytes(object),
        E::IndexAccess {
            object_name, index, ..
        } => object_name.capacity() as u64 + parsed_expression_bytes(index),
        E::ElementAccess { object, index, .. } | E::OptionalIndexAccess { object, index, .. } => {
            parsed_expression_bytes(object) + parsed_expression_bytes(index)
        }
        E::Call {
            callee_name,
            type_arguments,
            arguments,
            ..
        } => {
            callee_name.capacity() as u64
                + type_arguments.iter().map(parsed_type_bytes).sum::<u64>()
                + arguments
                    .iter()
                    .map(|argument| {
                        size_of::<surge_ts_syntax::ParsedCallArgument>() as u64
                            + parsed_expression_bytes(&argument.expression)
                    })
                    .sum::<u64>()
        }
        E::New {
            callee,
            type_arguments,
            arguments,
            ..
        }
        | E::OptionalCall {
            callee,
            type_arguments,
            arguments,
            ..
        } => {
            parsed_expression_bytes(callee)
                + type_arguments.iter().map(parsed_type_bytes).sum::<u64>()
                + arguments
                    .iter()
                    .map(|argument| {
                        size_of::<surge_ts_syntax::ParsedCallArgument>() as u64
                            + parsed_expression_bytes(&argument.expression)
                    })
                    .sum::<u64>()
        }
        E::PropertyCall {
            object,
            property_name,
            type_arguments,
            arguments,
            ..
        }
        | E::OptionalPropertyCall {
            object,
            property_name,
            type_arguments,
            arguments,
            ..
        } => {
            parsed_expression_bytes(object)
                + property_name.capacity() as u64
                + type_arguments.iter().map(parsed_type_bytes).sum::<u64>()
                + arguments
                    .iter()
                    .map(|argument| {
                        size_of::<surge_ts_syntax::ParsedCallArgument>() as u64
                            + parsed_expression_bytes(&argument.expression)
                    })
                    .sum::<u64>()
        }
        E::TypeAssertion { expression, ty, .. } => {
            parsed_expression_bytes(expression) + parsed_type_bytes(ty)
        }
        E::SatisfiesExpression {
            expression,
            target_type,
            ..
        } => parsed_expression_bytes(expression) + parsed_type_bytes(target_type),
        E::NonNullAssertion { expression, .. } | E::ConstAssertion { expression, .. } => {
            parsed_expression_bytes(expression)
        }
        E::JsxElement {
            tag_name,
            component_name,
            attributes,
            children,
            ..
        } => {
            tag_name.capacity() as u64
                + component_name
                    .as_ref()
                    .map_or(0, |name| name.capacity() as u64)
                + attributes
                    .iter()
                    .map(|attribute| {
                        size_of::<surge_ts_syntax::ParsedJsxAttribute>() as u64
                            + attribute.name.capacity() as u64
                            + attribute.value.as_ref().map_or(0, parsed_expression_bytes)
                    })
                    .sum::<u64>()
                + children.iter().map(parsed_jsx_child_bytes).sum::<u64>()
        }
        E::JsxFragment { children, .. } => children.iter().map(parsed_jsx_child_bytes).sum::<u64>(),
        E::ArrowFunction(arrow) => {
            let body = match &arrow.body {
                surge_ts_syntax::ParsedArrowFunctionBody::Expression(expression) => {
                    parsed_expression_bytes(expression)
                }
                surge_ts_syntax::ParsedArrowFunctionBody::Block(statements) => statements
                    .iter()
                    .map(parsed_body_statement_bytes)
                    .sum::<u64>(),
            };
            size_of::<surge_ts_syntax::ParsedArrowFunction>() as u64
                + string_vec_bytes(&arrow.body_reads)
                + arrow
                    .type_parameters
                    .iter()
                    .map(parsed_type_parameter_bytes)
                    .sum::<u64>()
                + arrow
                    .parameters
                    .iter()
                    .map(parsed_function_parameter_bytes)
                    .sum::<u64>()
                + arrow.return_type.as_ref().map_or(0, parsed_type_bytes)
                + body
        }
        _ => 0,
    }
}

fn parsed_jsx_child_bytes(child: &surge_ts_syntax::ParsedJsxChild) -> u64 {
    use surge_ts_syntax::ParsedJsxChild as C;
    size_of::<C>() as u64
        + match child {
            C::Expression { expression, .. } => {
                expression.as_ref().map_or(0, parsed_expression_bytes)
            }
            C::Element(element) => parsed_expression_bytes(element),
            _ => 0,
        }
}

fn string_vec_bytes(strings: &[String]) -> u64 {
    strings
        .iter()
        .map(|value| value.capacity() as u64 + size_of::<String>() as u64)
        .sum()
}

fn parsed_function_parameter_bytes(parameter: &surge_ts_syntax::ParsedFunctionParameter) -> u64 {
    size_of::<surge_ts_syntax::ParsedFunctionParameter>() as u64
        + parameter
            .declared_type
            .as_ref()
            .map_or(0, parsed_type_bytes)
        + parameter
            .initializer
            .as_ref()
            .map_or(0, parsed_expression_bytes)
}

fn parsed_body_statement_bytes(statement: &surge_ts_syntax::ParsedFunctionBodyStatement) -> u64 {
    use surge_ts_syntax::ParsedFunctionBodyStatement as B;
    let own = size_of::<B>() as u64;
    own + match statement {
        B::VariableDeclaration(declaration) => parsed_variable_declaration_bytes(declaration),
        B::Return(statement) => statement
            .expression
            .as_ref()
            .map_or(0, parsed_expression_bytes),
        B::Assignment(assignment) => {
            assignment.target_name.capacity() as u64 + parsed_expression_bytes(&assignment.value)
        }
        B::Expression(expression) => parsed_expression_bytes(expression),
        B::Block(statements) => statements.iter().map(parsed_body_statement_bytes).sum(),
        B::Function(function) => parsed_function_declaration_bytes(function),
        B::If(statement) => {
            parsed_expression_bytes(&statement.condition)
                + statement
                    .then_body
                    .iter()
                    .map(parsed_body_statement_bytes)
                    .sum::<u64>()
                + statement
                    .else_body
                    .iter()
                    .map(parsed_body_statement_bytes)
                    .sum::<u64>()
        }
        // Remaining variants (throw/while/for-of/switch/try/this-assignment)
        // carry the same expression/body shapes; their statement counts are
        // small enough that the enum-size charge above suffices for a census.
        _ => 0,
    }
}

fn parsed_variable_declaration_bytes(
    declaration: &surge_ts_syntax::ParsedVariableDeclaration,
) -> u64 {
    size_of::<surge_ts_syntax::ParsedVariableDeclaration>() as u64
        + declaration.name.capacity() as u64
        + declaration
            .declared_type
            .as_ref()
            .map_or(0, parsed_type_bytes)
        + declaration
            .initializer
            .as_ref()
            .map_or(0, parsed_expression_bytes)
}

fn parsed_function_declaration_bytes(function: &surge_ts_syntax::ParsedFunctionDeclaration) -> u64 {
    size_of::<surge_ts_syntax::ParsedFunctionDeclaration>() as u64
        + string_vec_bytes(&function.body_reads)
        + function.name.capacity() as u64
        + function
            .type_parameters
            .iter()
            .map(parsed_type_parameter_bytes)
            .sum::<u64>()
        + function
            .parameters
            .iter()
            .map(parsed_function_parameter_bytes)
            .sum::<u64>()
        + function.return_type.as_ref().map_or(0, parsed_type_bytes)
        + function
            .body
            .iter()
            .map(parsed_body_statement_bytes)
            .sum::<u64>()
}

fn parsed_statement_bytes(statement: &surge_ts_syntax::ParsedStatement) -> u64 {
    use surge_ts_syntax::ParsedStatement as S;
    let own = size_of::<S>() as u64;
    own + match statement {
        S::VariableDeclaration(declaration) => parsed_variable_declaration_bytes(declaration),
        S::Assignment(assignment) => {
            size_of::<surge_ts_syntax::ParsedAssignment>() as u64
                + assignment.target_name.capacity() as u64
                + parsed_expression_bytes(&assignment.value)
        }
        S::FunctionDeclaration(function) => parsed_function_declaration_bytes(function),
        S::Call(call) => {
            size_of::<surge_ts_syntax::ParsedCall>() as u64
                + call.callee_name.capacity() as u64
                + call
                    .type_arguments
                    .iter()
                    .map(parsed_type_bytes)
                    .sum::<u64>()
                + call
                    .arguments
                    .iter()
                    .map(|argument| {
                        size_of::<surge_ts_syntax::ParsedCallArgument>() as u64
                            + parsed_expression_bytes(&argument.expression)
                    })
                    .sum::<u64>()
        }
        S::MemberAssignment(assignment) => {
            size_of::<surge_ts_syntax::ParsedMemberAssignment>() as u64
                + parsed_expression_bytes(&assignment.target)
                + parsed_expression_bytes(&assignment.value)
        }
        S::Expression(expression) => parsed_expression_bytes(expression),
        S::TypeAliasDeclaration(alias) => {
            size_of::<surge_ts_syntax::ParsedTypeAliasDeclaration>() as u64
                + alias.name.capacity() as u64
                + alias
                    .type_parameters
                    .iter()
                    .map(parsed_type_parameter_bytes)
                    .sum::<u64>()
                + parsed_type_bytes(&alias.ty)
        }
        S::InterfaceDeclaration(interface) => {
            size_of::<surge_ts_syntax::ParsedInterfaceDeclaration>() as u64
                + interface.name.capacity() as u64
                + interface
                    .type_parameters
                    .iter()
                    .map(parsed_type_parameter_bytes)
                    .sum::<u64>()
                + interface
                    .extends
                    .iter()
                    .map(parsed_named_type_bytes)
                    .sum::<u64>()
                + interface
                    .members
                    .iter()
                    .map(parsed_interface_member_bytes)
                    .sum::<u64>()
                + interface
                    .string_index_type
                    .as_ref()
                    .map_or(0, parsed_type_bytes)
                + interface
                    .call_signature
                    .as_ref()
                    .map_or(0, parsed_function_type_bytes)
        }
        S::ClassDeclaration(class) => {
            use surge_ts_syntax::ParsedClassMember as M;
            size_of::<surge_ts_syntax::ParsedClassDeclaration>() as u64
                + class.name.capacity() as u64
                + class
                    .type_parameters
                    .iter()
                    .map(parsed_type_parameter_bytes)
                    .sum::<u64>()
                + class
                    .extends
                    .iter()
                    .map(parsed_named_type_bytes)
                    .sum::<u64>()
                + class
                    .members
                    .iter()
                    .map(|member| {
                        size_of::<M>() as u64
                            + match member {
                                M::Property(property) => {
                                    property.name.capacity() as u64
                                        + property
                                            .declared_type
                                            .as_ref()
                                            .map_or(0, parsed_type_bytes)
                                }
                                M::Method(method) => {
                                    method.name.capacity() as u64
                                        + string_vec_bytes(&method.body_reads)
                                        + method
                                            .parameters
                                            .iter()
                                            .map(parsed_function_parameter_bytes)
                                            .sum::<u64>()
                                        + method.return_type.as_ref().map_or(0, parsed_type_bytes)
                                        + method
                                            .body
                                            .iter()
                                            .map(parsed_body_statement_bytes)
                                            .sum::<u64>()
                                }
                                M::Accessor(accessor) => {
                                    accessor.name.capacity() as u64
                                        + accessor
                                            .getter_return_type
                                            .as_ref()
                                            .map_or(0, parsed_type_bytes)
                                        + accessor
                                            .setter_param_type
                                            .as_ref()
                                            .map_or(0, parsed_type_bytes)
                                }
                                M::StaticBlock(block) => {
                                    string_vec_bytes(&block.body_reads)
                                        + block
                                            .body
                                            .iter()
                                            .map(parsed_body_statement_bytes)
                                            .sum::<u64>()
                                }
                                M::Constructor(constructor) => {
                                    string_vec_bytes(&constructor.body_reads)
                                        + constructor
                                            .parameters
                                            .iter()
                                            .map(parsed_function_parameter_bytes)
                                            .sum::<u64>()
                                        + constructor
                                            .body
                                            .iter()
                                            .map(parsed_body_statement_bytes)
                                            .sum::<u64>()
                                }
                            }
                    })
                    .sum::<u64>()
        }
        S::ImportDeclaration(import) => {
            size_of::<surge_ts_syntax::ParsedImportDeclaration>() as u64
                + import.module_specifier.capacity() as u64
        }
        S::ExportDeclaration(export) => {
            size_of::<surge_ts_syntax::ParsedExportDeclaration>() as u64
                + match export.as_ref() {
                    surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } => {
                        parsed_statement_bytes(declaration)
                    }
                    _ => 0,
                }
        }
        S::DeclareModuleDeclaration(declaration) => {
            size_of::<surge_ts_syntax::ParsedDeclareModuleDeclaration>() as u64
                + declaration.module_specifier.capacity() as u64
                + declaration
                    .statements
                    .iter()
                    .map(parsed_statement_bytes)
                    .sum::<u64>()
        }
        S::NamespaceDeclaration(namespace) => {
            size_of::<surge_ts_syntax::ParsedNamespaceDeclaration>() as u64
                + namespace.name.capacity() as u64
                + namespace
                    .statements
                    .iter()
                    .map(parsed_statement_bytes)
                    .sum::<u64>()
        }
        S::Block(statements) => statements
            .iter()
            .map(parsed_body_statement_bytes)
            .sum::<u64>(),
        S::If(if_statement) => {
            size_of::<surge_ts_syntax::ParsedIfStatement>() as u64
                + parsed_expression_bytes(&if_statement.condition)
                + if_statement
                    .then_body
                    .iter()
                    .chain(&if_statement.else_body)
                    .map(parsed_body_statement_bytes)
                    .sum::<u64>()
        }
        S::UnsupportedDeclaration { .. } => 0,
    }
}

pub(crate) fn emit_retention_census(
    stage: &str,
    ctx: Option<&CheckerContext>,
    store: &Arc<surge_ts_types::ProgramTypeStore>,
    view: RetentionCensusView<'_>,
) {
    if !retention_census_enabled() {
        return;
    }

    let mut walker = Walker::new();

    if let Some(analyses) = view.module_analyses {
        walker.current_group = "module_analyses.local_symbols";
        for analysis in analyses.iter().flatten() {
            walker.walk_symbol_table(analysis.local_symbols());
        }
        walker.current_group = "module_analyses.export_tables";
        for analysis in analyses.iter().flatten() {
            walker.walk_export_table(analysis.local_export_table());
        }
        walker.current_group = "module_analyses.decl_tables";
        for analysis in analyses.iter().flatten() {
            walker.walk_declaration_table(analysis.local_type_declarations());
        }
    }
    if let Some(parsed_files) = view.parsed_files {
        walker.current_group = "parsed_files";
        for file in parsed_files {
            let mut bytes = file.file_name.capacity() as u64
                + file
                    .module_reads
                    .iter()
                    .map(|read| read.capacity() as u64 + size_of::<String>() as u64)
                    .sum::<u64>()
                + (file.statements.capacity() * size_of::<surge_ts_syntax::ParsedStatement>())
                    as u64;
            for statement in &file.statements {
                bytes += parsed_statement_bytes(statement);
            }
            walker.add(bytes);
            walker.add_item();
        }
    }
    if let Some(analyses) = view.preliminary_module_analyses {
        walker.current_group = "preliminary_analyses";
        for analysis in analyses.iter().flatten() {
            walker.walk_symbol_table(analysis.local_symbols());
            walker.walk_export_table(analysis.local_export_table());
            walker.walk_declaration_table(analysis.local_type_declarations());
        }
    }
    if let Some(bindings) = view.module_import_bindings {
        walker.current_group = "import_bindings";
        for binding in bindings.iter().flatten() {
            walker.walk_declaration_table(&binding.type_declarations);
            walker.walk_symbol_table(&binding.symbols);
            for layer in &binding.namespace_alias_layers {
                walker.walk_declaration_table(layer);
            }
        }
    }
    if let Some(bindings) = view.preliminary_module_import_bindings {
        walker.current_group = "preliminary_import_bindings";
        for binding in bindings.iter().flatten() {
            walker.walk_declaration_table(&binding.type_declarations);
            walker.walk_symbol_table(&binding.symbols);
            for layer in &binding.namespace_alias_layers {
                walker.walk_declaration_table(layer);
            }
        }
    }
    if let Some(scopes) = view.module_resolution_scopes {
        walker.current_group = "resolution_scopes";
        for scope in scopes.iter().flatten() {
            walker.walk_scope(scope);
        }
    }
    if let Some(globals) = view.global_symbols {
        walker.current_group = "global_symbols";
        walker.walk_symbol_table(globals);
    }
    if let Some(signatures) = view.function_signatures {
        walker.current_group = "global_function_signatures";
        for function in signatures {
            walker.walk_function(function);
        }
    }

    if let Some(ctx) = ctx {
        let snap = |walker: &Walker| {
            (
                walker.parsed_signature_bytes,
                walker.resolver_own_bytes,
                walker.resolver_shared_bytes,
                walker.declaration_parsed_bytes,
                walker.declaration_index_bytes,
                walker.resolver_count,
                walker.reference_count,
            )
        };
        let report = |label: &str, before: (u64, u64, u64, u64, u64, u64, u64), walker: &Walker| {
            let after = snap(walker);
            eprintln!(
                "  DETAIL {label}: parsed_sig={} resolver_own={} resolver_shared={} decl_parsed={} decl_index={} resolvers={} references={}",
                after.0 - before.0,
                after.1 - before.1,
                after.2 - before.2,
                after.3 - before.3,
                after.4 - before.4,
                after.5 - before.5,
                after.6 - before.6,
            );
        };
        let before = snap(&walker);
        walker.current_group = "ctx.ambient_globals.symbols";
        walker.walk_symbol_table(&ctx.ambient_global_symbols);
        report("ambient_globals.symbols", before, &walker);
        let before = snap(&walker);
        walker.current_group = "ctx.ambient_globals.live_decls";
        walker.walk_declaration_table(&ctx.type_declarations);
        report("ambient_globals.live_decls", before, &walker);
        let before = snap(&walker);
        walker.current_group = "ctx.ambient_globals.global_decls";
        walker.walk_declaration_table(&ctx.ambient_global_type_declarations);
        report("ambient_globals.global_decls", before, &walker);
        walker.current_group = "ctx.ambient_modules";
        for table in ctx.ambient_modules.values() {
            walker.walk_export_table(table);
        }
        for table in ctx.module_augmentations.values() {
            walker.walk_export_table(table);
        }
        walker.current_group = "ctx.module_scope_by_file";
        for scope in ctx.module_scope_by_file.values() {
            walker.walk_scope(scope);
        }
        walker.current_group = "ctx.module_local_values";
        for table in ctx.module_local_values_by_file.values() {
            walker.walk_symbol_table(table);
        }
        ctx.declaration_environment_store.census_environments(
            &mut |file_name, symbols, type_declarations, scope, type_parameter_entries| {
                walker.environment_count += 1;
                let own_bytes = file_name.len() as u64 + 512 + (type_parameter_entries * 64) as u64;
                walker.environment_index_bytes += own_bytes;
                walker.current_group = "declaration_environments.own";
                walker.add(own_bytes);
                walker.current_group = "declaration_environments.symbols";
                walker.walk_symbol_table(symbols);
                walker.current_group = "declaration_environments.snapshots";
                walker.walk_declaration_table(type_declarations);
                if let Some(scope) = scope {
                    walker.current_group = "declaration_environments.scopes";
                    walker.walk_scope(scope);
                }
            },
        );
        walker.current_group = "ctx.resolved_named_types";
        if let Ok(cache) = ctx.resolved_named_types.lock() {
            for state in cache.values() {
                if let DeclarationResolutionState::Resolved { ty, .. } = state {
                    walker.walk_type(ty);
                }
            }
        }
        walker.current_group = "ctx.instantiation_caches";
        if let Ok(cache) = ctx.program_instantiations.lock() {
            for bucket in cache.values() {
                for entry in bucket {
                    for argument in &entry.arguments {
                        walker.walk_type(argument);
                    }
                    walker.walk_type(&entry.resolved);
                }
            }
        }
        if let Ok(cache) = ctx.program_resolved_generic_types.lock() {
            for bucket in cache.values() {
                for entry in bucket {
                    for argument in &entry.arguments {
                        walker.walk_type(argument);
                    }
                    walker.walk_type(&entry.ty);
                }
            }
        }
        walker.current_group = "ctx.physical_interface_caches";
        if let Ok(cache) = ctx.physical_interface_instantiations.lock() {
            for ty in cache.values() {
                walker.walk_type(ty);
            }
        }
        if let Ok(cache) = ctx.physical_interface_method_instantiations.lock() {
            for function in cache.values() {
                walker.walk_function(function);
            }
        }
        if let Ok(cache) = ctx.physical_interface_overload_instantiations.lock() {
            for function in cache.values() {
                walker.walk_function(function);
            }
        }
    }

    // Walk store-owned payloads not already reached from live owners so the
    // store's full retention is visible.
    walker.current_group = "canonical_store_unreached";
    let store_census = store.retained_census();
    store.for_each_retained_type(&mut |ty| walker.walk_type(ty));

    let mut groups: Vec<(&'static str, GroupTally)> =
        walker.groups.iter().map(|(k, v)| (*k, *v)).collect();
    groups.sort_by_key(|(_, tally)| std::cmp::Reverse(tally.bytes));
    let footprint = super::current_footprint_bytes();
    eprintln!(
        "RETENTION CENSUS stage={stage} footprint={} store_functions={} store_parameter_elements={} store_union_members={} store_property_entries={}",
        footprint.map_or_else(
            || "n/a".into(),
            |b| format!("{:.2}GB", b as f64 / (1 << 30) as f64)
        ),
        store_census.function_payloads,
        store_census.parameter_list_elements,
        store_census.union_member_elements,
        store_census.property_map_entries,
    );
    for (name, tally) in &groups {
        eprintln!(
            "  group {name:<38} bytes={:>12} items={}",
            tally.bytes, tally.items
        );
        if let Some(subs) = walker.sub_tallies.get(name) {
            let mut subs: Vec<(&&'static str, &u64)> = subs.iter().collect();
            subs.sort_by_key(|(_, bytes)| std::cmp::Reverse(**bytes));
            for (key, bytes) in subs {
                eprintln!("    sub {key:<36} bytes={bytes:>12}");
            }
        }
    }
    eprintln!(
        "  functions: canonical={} fallback={} fallback_bytes={} classes: unknown={} ctx_ref={} over_budget={} internable={}",
        walker.canonical_function_payloads,
        walker.fallback_function_payloads,
        walker.fallback_function_bytes,
        walker.fallback_classes.unknown_containing,
        walker.fallback_classes.context_retaining_reference,
        walker.fallback_classes.over_budget,
        walker.fallback_classes.internable,
    );
    eprintln!(
        "  symbols={} parsed_signature_bytes={} declarations={} declaration_parsed_bytes={} span_map_entries={} span_map_bytes={} references={} reference_argument_slots={}",
        walker.symbol_count,
        walker.parsed_signature_bytes,
        walker.declaration_entries,
        walker.declaration_parsed_bytes,
        walker.span_map_entries,
        walker.span_map_bytes,
        walker.reference_count,
        walker.reference_argument_slots,
    );
    eprintln!(
        "  resolvers={} resolver_own_bytes={} resolver_shared_bytes={} environments={} environment_own_bytes={} decl_table_instances={} decl_index_bytes={}",
        walker.resolver_count,
        walker.resolver_own_bytes,
        walker.resolver_shared_bytes,
        walker.environment_count,
        walker.environment_index_bytes,
        walker.declaration_table_instances,
        walker.declaration_index_bytes,
    );
    eprintln!(
        "  scope_attached_decls={} scope_self_cycles={}",
        walker.scope_attached_declarations, walker.scope_self_cycle_declarations,
    );
}
