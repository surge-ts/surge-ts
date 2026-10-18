use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use surge_ts_syntax::ParsedType;
use surge_ts_types::fx::{FxHashMap, FxHashSet};
use surge_ts_types::{FunctionType, ProgramTypeStore, Type};

use crate::infer::types::LazyMemberTemplateTable;
use crate::program::ProgramTimings;
use crate::symbols::{
    SymbolTable, TypeDeclarationScope, TypeDeclarationTable,
};

use crate::modules::ModuleExportTable;
use super::{
    CheckerContext, CheckerOptions, DeclarationResolutionKey, DeclarationResolutionState,
    FileKind, GenericInstantiationCacheEntry, InstantiationCacheEntry,
    InterfaceDeclarationTemplate, InterfaceInstantiationKey, InterfaceMemberInstantiationKey,
    InterfaceOverloadInstantiationKey, StableInterfaceDeclarationId, SubstitutionStore,
};

pub(super) static NEXT_DECLARATION_ENVIRONMENT_OWNER: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DeclarationEnvironmentId(u64);

impl DeclarationEnvironmentId {
    pub(super) fn new(owner: u32, index: u32) -> Self {
        Self((u64::from(owner) << 32) | u64::from(index))
    }

    pub(super) fn owner(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub(super) fn index(self) -> usize {
        (self.0 as u32).saturating_sub(1) as usize
    }
}

/// Content-stable identity of one `resolved_named_types` memo map instance.
/// Pointer identity is regime-dependent (a parallel worker's fresh context
/// creates different map instances than the serial rolling context), so map
/// instances are identified by *where the program created them*: the file
/// whose body window created the map, the deterministic resolution-stage
/// counter, the within-body ordinal (body start / mid-body / shadow), and the
/// speculative-attempt tag (0 = first attempt, 1 = STC recheck, which must
/// not collide with the discarded speculative attempt's environments).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct EnvironmentMapIdentity {
    pub(super) creator: Arc<str>,
    pub(super) stage: u64,
    pub(super) ordinal: u32,
    pub(super) attempt: u64,
}

impl EnvironmentMapIdentity {
    pub(super) fn initial() -> Self {
        Self {
            creator: Arc::from(""),
            stage: 0,
            ordinal: 0,
            attempt: 0,
        }
    }
}

/// Dedup key for interned declaration environments. Every component is
/// content-derived (never a bare allocation address), so two contexts in the
/// same semantic state — a parallel worker's fresh clone and the serial
/// rolling context at the same module — intern to the same environment and,
/// critically, produce the same canonicalization discriminator: lazy
/// `Type::Reference`s compare equal across regimes exactly when serial
/// semantics say they should (see `program_canonicalization_discriminator`
/// consumers in the canonical type store).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct DeclarationEnvironmentKey {
    pub(super) file_name: String,
    pub(super) file_kind: FileKind,
    /// Captured inside another file's declaration (`cross_file_resolution_depth
    /// > 0`), where the live table is the consumer's and must not answer names.
    pub(super) crossed_file: bool,
    /// `(instance_id, version)` per scope layer, in layer order; `None` scope
    /// is distinguished from an empty layer list.
    pub(super) has_scope: bool,
    pub(super) scope_layers: Vec<(u64, u64)>,
    pub(super) resolved_named_types_identity: EnvironmentMapIdentity,
    /// The per-file module maps are stage-installed shared `Arc`s (regime
    /// stable); an empty map is identified as 0 regardless of which default
    /// `Arc` instance holds it (`mem::take` windows create fresh empties).
    pub(super) module_scope_identity: usize,
    pub(super) module_values_identity: usize,
    /// The context's live declaration table, by the map it shares (see
    /// [`environment_table_identity`]). The content key must carry it, or
    /// environments over different tables merge and first-wins data capture
    /// materializes the wrong table.
    pub(super) type_declarations_identity: (u64, u64),
    /// Deterministic stage counter at intern time: distinguishes re-visits of
    /// a file across stage boundaries that share a carried memo map.
    pub(super) stage_at_intern: u64,
    /// File-switch ordinal within the current anchor window (window = last
    /// memo-map replacement or stage boundary): the regime-stable
    /// reconstruction of the old rolling generation, which separated fixpoint
    /// re-entries of the same file — observable through which re-entry's
    /// references canonicalize together.
    pub(super) visit: u64,
}

/// The identity of a context's live declaration table: the address of the
/// map it shares. Two tables over one map have one content, and every key
/// that names a map belongs to an environment whose captured snapshot holds
/// that map, so the map cannot change in place (`Arc::make_mut` copies a
/// shared map) or be freed and its address reused while the key lives.
/// `(instance_id, version)` told apart every clone of a table instead, and a
/// context recovered from an environment clones its table, so each lazy
/// reference made while resolving in one interned an environment of its own
/// (241K distinct keys on trpc against 13.7K without that component).
fn environment_table_identity(table: &crate::symbols::TypeDeclarationTable) -> (u64, u64) {
    (table.identity_address() as u64, 0)
}

thread_local! {
    /// One-entry front cache for [`DeclarationEnvironmentStore::intern`]. Held
    /// per thread rather than on the context because `CheckerContext` must stay
    /// `Sync`; a stale entry is impossible because the hit path compares the
    /// cached key field-by-field against the live context, so an entry filled
    /// by a different context can only miss. The store owner is carried so two
    /// programs on one thread cannot trade identities.
    static DECLARATION_ENVIRONMENT_MEMO: RefCell<
        Option<(u32, DeclarationEnvironmentKey, DeclarationEnvironmentId, u64)>,
    > = const { RefCell::new(None) };
}

impl DeclarationEnvironmentKey {
    /// Whether this key is exactly what `intern` would build for `ctx` right
    /// now. Every field of the struct is compared, in cheapest-first order, so
    /// a hit is equivalent to building the key and finding it in the store —
    /// but without the `String`/`Vec` allocations or the store lock. Keep this
    /// in sync with the key construction in `DeclarationEnvironmentStore::intern`.
    pub(super) fn matches(&self, ctx: &CheckerContext) -> bool {
        if self.file_kind != ctx.current_file_kind
            || self.crossed_file != (ctx.cross_file_resolution_depth > 0)
            || self.stage_at_intern != ctx.resolution_stage_counter
            || self.visit != ctx.environment_visit_counter
            || self.type_declarations_identity != environment_table_identity(&ctx.type_declarations)
            || self.has_scope != ctx.type_declaration_scope.is_some()
        {
            return false;
        }
        let module_scope_identity = if ctx.module_scope_by_file.is_empty() {
            0
        } else {
            Arc::as_ptr(&ctx.module_scope_by_file) as usize
        };
        let module_values_identity = if ctx.module_local_values_by_file.is_empty() {
            0
        } else {
            Arc::as_ptr(&ctx.module_local_values_by_file) as usize
        };
        if self.module_scope_identity != module_scope_identity
            || self.module_values_identity != module_values_identity
            || self.resolved_named_types_identity != ctx.resolved_named_types_identity
        {
            return false;
        }
        let layers_match = match ctx.type_declaration_scope.as_ref() {
            Some(scope) => {
                let layers = scope.layers();
                layers.len() == self.scope_layers.len()
                    && layers
                        .iter()
                        .zip(self.scope_layers.iter())
                        .all(|(layer, cached)| layer.snapshot_identity() == *cached)
            }
            None => self.scope_layers.is_empty(),
        };
        layers_match && self.file_name == ctx.file_name
    }
}

pub(super) fn environment_content_discriminator(key: &DeclarationEnvironmentKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = surge_ts_types::fx::FxHasher::default();
    key.hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone)]
pub(super) struct DeclarationEnvironmentData {
    pub(super) file_name: String,
    pub(super) crossed_file: bool,
    pub(super) current_file_kind: FileKind,
    pub(super) options: Arc<CheckerOptions>,
    pub(super) symbols: SymbolTable,
    /// Immutable snapshot of the capturing context's `type_declarations`.
    /// Shared across every environment interned while the live table is
    /// unmutated (see [`DeclarationEnvironmentStore::snapshot_type_declarations`]);
    /// tens of thousands of environments would otherwise each own a full index
    /// copy of their module's declaration table.
    pub(super) type_declarations: Arc<TypeDeclarationTable>,
    pub(super) type_declaration_scope: Option<Arc<TypeDeclarationScope>>,
    pub(super) program_type_store: Arc<ProgramTypeStore>,
    pub(super) substitution_store: Arc<SubstitutionStore>,
    pub(super) resolved_named_types:
        Arc<Mutex<FxHashMap<DeclarationResolutionKey, DeclarationResolutionState>>>,
    pub(super) program_resolved_generic_types:
        Arc<Mutex<FxHashMap<DeclarationResolutionKey, Vec<GenericInstantiationCacheEntry>>>>,
    pub(super) program_instantiations:
        Arc<Mutex<FxHashMap<DeclarationResolutionKey, Vec<InstantiationCacheEntry>>>>,
    pub(super) physical_interface_instantiations: Arc<Mutex<FxHashMap<InterfaceInstantiationKey, Arc<Type>>>>,
    pub(super) physical_interface_declaration_templates:
        Arc<Mutex<FxHashMap<StableInterfaceDeclarationId, Arc<InterfaceDeclarationTemplate>>>>,
    pub(super) physical_interface_method_instantiations:
        Arc<Mutex<FxHashMap<InterfaceMemberInstantiationKey, FunctionType>>>,
    pub(super) physical_interface_overload_instantiations:
        Arc<Mutex<FxHashMap<InterfaceOverloadInstantiationKey, FunctionType>>>,
    pub(super) lazy_member_annotation_templates: Arc<Mutex<LazyMemberTemplateTable>>,
    pub(super) ambient_modules: Arc<FxHashMap<String, ModuleExportTable>>,
    pub(super) import_type_namespaces: Arc<Mutex<FxHashMap<(Arc<str>, String), super::ImportTypeTarget>>>,
    pub(super) import_type_globals: Arc<Mutex<FxHashSet<String>>>,
    pub(super) ambient_file_type_scopes: Arc<FxHashMap<Arc<str>, Arc<TypeDeclarationScope>>>,
    pub(super) module_augmentations: Arc<FxHashMap<String, ModuleExportTable>>,
    pub(super) ambient_global_symbols: SymbolTable,
    pub(super) ambient_global_type_declarations: Arc<TypeDeclarationTable>,
    pub(super) module_file_index_by_identity: Arc<FxHashMap<Arc<str>, usize>>,
    pub(super) module_scope_by_file: Arc<FxHashMap<Arc<str>, Arc<TypeDeclarationScope>>>,
    pub(super) module_local_values_by_file: Arc<FxHashMap<Arc<str>, Arc<SymbolTable>>>,
    pub(super) jsx_namespace_modules: Arc<crate::checks::jsx::JsxNamespaceModules>,
    pub(super) type_parameter_scopes: Vec<Arc<HashMap<String, Type>>>,
    pub(super) type_parameter_constraint_scopes: Vec<Arc<HashMap<String, ParsedType>>>,
    pub(super) timings: Option<Arc<Mutex<ProgramTimings>>>,
    pub(super) file_kinds: Arc<FxHashMap<String, FileKind>>,
    pub(super) module_value_fallback: Option<Arc<SymbolTable>>,
    pub(super) resolved_named_types_identity: EnvironmentMapIdentity,
    pub(super) resolution_stage_counter: u64,
    pub(super) environment_attempt: u64,
    pub(super) environment_visit_counter: u64,
}

#[derive(Debug)]
pub(crate) struct DeclarationEnvironmentStore {
    pub(super) owner: u32,
    /// Set on the program root's store only. A shadow context (`values.rs`)
    /// mints its own store, which dies with the shadow; a reference captured
    /// into it degrades to `Unknown` on every later peel. Program-lifetime
    /// caches must not accept a value produced under such a store.
    program_lifetime: std::sync::atomic::AtomicBool,
    pub(super) next_index: AtomicU32,
    pub(super) requests: AtomicU64,
    pub(super) hits: AtomicU64,
    pub(super) entries: Mutex<DeclarationEnvironmentEntries>,
    /// `(instance_id, version)` → snapshot memo for the most recent
    /// `type_declarations` capture. Environments are interned in bursts between
    /// table mutations, so one snapshot serves the whole burst.
    pub(super) type_declarations_snapshot: Mutex<Option<((u64, u64), Arc<TypeDeclarationTable>)>>,
    /// The program's current module-local value tables. An environment snapshots
    /// the table it was captured under, and one captured during module analysis
    /// predates the final table; a lazy body return forced in the check phase
    /// reads the current one here. `Weak`, so the store never extends its life.
    pub(super) module_local_values: Mutex<ModuleLocalValuesByFile>,
}

type ModuleLocalValuesByFile = Weak<FxHashMap<Arc<str>, Arc<SymbolTable>>>;

#[derive(Debug, Default)]
pub(super) struct DeclarationEnvironmentEntries {
    pub(super) by_key: HashMap<DeclarationEnvironmentKey, (DeclarationEnvironmentId, u64)>,
    pub(super) by_id: Vec<Arc<DeclarationEnvironmentData>>,
}

#[derive(Debug, Clone)]
pub(crate) struct DeclarationEnvironmentHandle {
    pub(super) id: DeclarationEnvironmentId,
    /// Content hash of the environment's dedup key: equal for semantically
    /// identical environments regardless of context instance or intern order.
    pub(super) discriminator: u64,
    pub(super) store: Weak<DeclarationEnvironmentStore>,
}

impl DeclarationEnvironmentStore {
    pub(crate) fn mark_program_lifetime(self: &Arc<Self>) {
        self.program_lifetime
            .store(true, std::sync::atomic::Ordering::Relaxed);
        *PROGRAM_LIFETIME_STORE.lock().unwrap_or_else(|error| error.into_inner()) = Arc::downgrade(self);
    }

    pub(crate) fn is_program_lifetime(&self) -> bool {
        self.program_lifetime
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) fn new() -> Arc<Self> {
        let owner = NEXT_DECLARATION_ENVIRONMENT_OWNER.fetch_add(1, Ordering::Relaxed);
        assert_ne!(owner, 0, "declaration-environment owner space exhausted");
        Arc::new(Self {
            owner,
            program_lifetime: std::sync::atomic::AtomicBool::new(false),
            next_index: AtomicU32::new(1),
            requests: AtomicU64::new(0),
            hits: AtomicU64::new(0),
            entries: Mutex::new(DeclarationEnvironmentEntries::default()),
            type_declarations_snapshot: Mutex::new(None),
            module_local_values: Mutex::new(Weak::new()),
        })
    }

    /// The per-file module value tables as the program last published them. A
    /// context recovered from an environment holds the map it was captured
    /// with, which can predate the final import bindings.
    pub(crate) fn published_module_local_values(
        &self,
    ) -> Option<Arc<FxHashMap<Arc<str>, Arc<SymbolTable>>>> {
        self.module_local_values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .upgrade()
    }

    pub(super) fn publish_module_local_values(
        &self,
        values: &Arc<FxHashMap<Arc<str>, Arc<SymbolTable>>>,
    ) {
        *self
            .module_local_values
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Arc::downgrade(values);
    }

    /// Returns a shared immutable snapshot of `ctx.type_declarations`, reusing
    /// the previous snapshot while the exact same table instance is unmutated.
    pub(super) fn snapshot_type_declarations(&self, ctx: &CheckerContext) -> Arc<TypeDeclarationTable> {
        let identity = environment_table_identity(&ctx.type_declarations);
        let mut memo = self
            .type_declarations_snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((cached_identity, snapshot)) = memo.as_ref()
            && *cached_identity == identity
        {
            return snapshot.clone();
        }
        let snapshot = Arc::new(ctx.type_declarations.clone());
        *memo = Some((identity, snapshot.clone()));
        snapshot
    }

    pub(super) fn intern(self: &Arc<Self>, ctx: &CheckerContext) -> DeclarationEnvironmentHandle {
        self.requests.fetch_add(1, Ordering::Relaxed);
        // One-entry front cache. `declaration_environment()` is called once per
        // lazy-reference creation (487k times on tanstack-query) and almost
        // always lands on the same environment as the previous call, but
        // building the key allocates a `String` and a `Vec` and consulting the
        // store takes a global lock. The memo holds the exact key the store
        // would have hashed, so a hit is indistinguishable from an intern.
        if let Some((id, discriminator)) = DECLARATION_ENVIRONMENT_MEMO.with(|memo| {
            memo.borrow()
                .as_ref()
                .filter(|(owner, key, _, _)| *owner == self.owner && key.matches(ctx))
                .map(|(_, _, id, discriminator)| (*id, *discriminator))
        }) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return DeclarationEnvironmentHandle {
                id,
                discriminator,
                store: Arc::downgrade(self),
            };
        }
        let key = DeclarationEnvironmentKey {
            file_name: ctx.file_name.clone(),
            file_kind: ctx.current_file_kind,
            crossed_file: ctx.cross_file_resolution_depth > 0,
            has_scope: ctx.type_declaration_scope.is_some(),
            scope_layers: ctx
                .type_declaration_scope
                .as_ref()
                .map_or_else(Vec::new, |scope| {
                    scope
                        .layers()
                        .iter()
                        .map(|layer| layer.snapshot_identity())
                        .collect()
                }),
            resolved_named_types_identity: ctx.resolved_named_types_identity.clone(),
            module_scope_identity: if ctx.module_scope_by_file.is_empty() {
                0
            } else {
                Arc::as_ptr(&ctx.module_scope_by_file) as usize
            },
            module_values_identity: if ctx.module_local_values_by_file.is_empty() {
                0
            } else {
                Arc::as_ptr(&ctx.module_local_values_by_file) as usize
            },
            type_declarations_identity: environment_table_identity(&ctx.type_declarations),
            stage_at_intern: ctx.resolution_stage_counter,
            visit: ctx.environment_visit_counter,
        };
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((id, discriminator)) = entries.by_key.get(&key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            let (id, discriminator) = (*id, *discriminator);
            drop(entries);
            DECLARATION_ENVIRONMENT_MEMO
                .with(|memo| *memo.borrow_mut() = Some((self.owner, key, id, discriminator)));
            return DeclarationEnvironmentHandle {
                id,
                discriminator,
                store: Arc::downgrade(self),
            };
        }
        let id = DeclarationEnvironmentId::new(
            self.owner,
            self.next_index.fetch_add(1, Ordering::Relaxed),
        );
        let discriminator = environment_content_discriminator(&key);
        let data = Arc::new(DeclarationEnvironmentData::capture(
            ctx,
            self.snapshot_type_declarations(ctx),
        ));
        debug_assert_eq!(id.index(), entries.by_id.len());
        entries.by_key.insert(key.clone(), (id, discriminator));
        entries.by_id.push(data);
        drop(entries);
        DECLARATION_ENVIRONMENT_MEMO
            .with(|memo| *memo.borrow_mut() = Some((self.owner, key, id, discriminator)));
        DeclarationEnvironmentHandle {
            id,
            discriminator,
            store: Arc::downgrade(self),
        }
    }

    pub(crate) fn stats(&self) -> (u64, u64, u64) {
        (
            self.requests.load(Ordering::Relaxed),
            self.hits.load(Ordering::Relaxed),
            self.entries
                .lock()
                .map(|entries| entries.by_id.len() as u64)
                .unwrap_or_default(),
        )
    }

    /// Census-only iteration over the interned environments, exposing the
    /// owned captures a retained-memory walk needs to attribute.
    pub(crate) fn census_environments(
        &self,
        f: &mut dyn FnMut(
            &str,
            &SymbolTable,
            &TypeDeclarationTable,
            Option<&Arc<TypeDeclarationScope>>,
            usize,
        ),
    ) {
        let Ok(entries) = self.entries.lock() else {
            return;
        };
        for data in &entries.by_id {
            let type_parameter_scope_entries = data
                .type_parameter_scopes
                .iter()
                .map(|scope| scope.len())
                .sum::<usize>()
                + data
                    .type_parameter_constraint_scopes
                    .iter()
                    .map(|scope| scope.len())
                    .sum::<usize>();
            f(
                &data.file_name,
                &data.symbols,
                &data.type_declarations,
                data.type_declaration_scope.as_ref(),
                type_parameter_scope_entries,
            );
        }
    }
}

/// The store of the program being checked, held weakly: it lives as long as
/// the program's own context does.
static PROGRAM_LIFETIME_STORE: Mutex<Weak<DeclarationEnvironmentStore>> = Mutex::new(Weak::new());

/// A context recovered from the program's own store, for a reference whose
/// capturing store is gone — a shadow context's, which does not share the
/// program's (see `collect_exportable_value_symbols`). Only an expansion that
/// does not depend on where the reference was written may use it.
pub(crate) fn program_checker_context() -> Option<CheckerContext> {
    let store = PROGRAM_LIFETIME_STORE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .upgrade()?;
    let data = store.entries.lock().ok()?.by_id.first().cloned()?;
    Some(CheckerContext::from_declaration_environment(&data, store))
}

impl DeclarationEnvironmentHandle {
    pub(crate) fn checker_context(&self) -> Option<CheckerContext> {
        let store = self.store.upgrade()?;
        if self.id.owner() != store.owner {
            return None;
        }
        let data = store
            .entries
            .lock()
            .ok()?
            .by_id
            .get(self.id.index())
            .cloned()?;
        Some(CheckerContext::from_declaration_environment(&data, store))
    }

    /// The module value tables as the program last published them, read
    /// without recovering a context.
    pub(crate) fn published_module_local_values(
        &self,
    ) -> Option<Arc<FxHashMap<Arc<str>, Arc<SymbolTable>>>> {
        self.store.upgrade()?.published_module_local_values()
    }

    /// The declaring file's module-local values as the program holds them now.
    pub(crate) fn current_module_local_values(&self, file_name: &str) -> Option<Arc<SymbolTable>> {
        let store = self.store.upgrade()?;
        let values = store
            .module_local_values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .upgrade()?;
        values.get(file_name).cloned()
    }

    pub(crate) fn canonicalization_discriminator(&self) -> u64 {
        self.discriminator
    }
}

impl DeclarationEnvironmentData {
    pub(super) fn capture(ctx: &CheckerContext, type_declarations: Arc<TypeDeclarationTable>) -> Self {
        Self {
            file_name: ctx.file_name.clone(),
            crossed_file: ctx.cross_file_resolution_depth > 0,
            current_file_kind: ctx.current_file_kind,
            options: ctx.options.clone(),
            // EXPERIMENT(env-symbols): drop the working value-table capture;
            // typeof falls back to ambient globals / module_value_fallback /
            // module_local_values_by_file.
            symbols: SymbolTable::empty_for_environment_capture(),
            type_declarations,
            type_declaration_scope: ctx.type_declaration_scope.clone(),
            program_type_store: ctx.program_type_store.clone(),
            substitution_store: ctx.substitution_store.clone(),
            resolved_named_types: ctx.resolved_named_types.clone(),
            program_resolved_generic_types: ctx.program_resolved_generic_types.clone(),
            program_instantiations: ctx.program_instantiations.clone(),
            physical_interface_instantiations: ctx.physical_interface_instantiations.clone(),
            lazy_member_annotation_templates: ctx.lazy_member_annotation_templates.clone(),
            physical_interface_declaration_templates: ctx
                .physical_interface_declaration_templates
                .clone(),
            physical_interface_method_instantiations: ctx
                .physical_interface_method_instantiations
                .clone(),
            physical_interface_overload_instantiations: ctx
                .physical_interface_overload_instantiations
                .clone(),
            ambient_modules: ctx.ambient_modules.clone(),
            import_type_namespaces: ctx.import_type_namespaces.clone(),
            import_type_globals: ctx.import_type_globals.clone(),
            ambient_file_type_scopes: ctx.ambient_file_type_scopes.clone(),
            module_augmentations: ctx.module_augmentations.clone(),
            ambient_global_symbols: ctx.ambient_global_symbols.clone_for_environment_capture(),
            ambient_global_type_declarations: ctx.ambient_global_type_declarations.clone(),
            module_file_index_by_identity: ctx.module_file_index_by_identity.clone(),
            module_scope_by_file: ctx.module_scope_by_file.clone(),
            module_local_values_by_file: ctx.module_local_values_by_file.clone(),
            jsx_namespace_modules: ctx.jsx_namespace_modules.clone(),
            type_parameter_scopes: ctx.type_parameter_scopes.clone(),
            type_parameter_constraint_scopes: ctx.type_parameter_constraint_scopes.clone(),
            timings: ctx.timings.clone(),
            file_kinds: ctx.file_kinds.clone(),
            resolved_named_types_identity: ctx.resolved_named_types_identity.clone(),
            resolution_stage_counter: ctx.resolution_stage_counter,
            environment_attempt: ctx.environment_attempt,
            environment_visit_counter: ctx.environment_visit_counter,
            module_value_fallback: ctx.module_value_fallback.clone(),
        }
    }
}

