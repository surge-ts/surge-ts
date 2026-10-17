use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use surge_ts_types::fx::{FxHashMap, FxHashSet};
use surge_ts_types::{FunctionType, Type};

use crate::context::{
    DeclarationResolutionKey, GenericInstantiationCacheEntry,
    InstantiationCacheEntry, InterfaceDeclarationTemplate, InterfaceInstantiationKey,
    InterfaceMemberInstantiationKey, InterfaceOverloadInstantiationKey,
    StableInterfaceDeclarationId,
};
use super::{
    CacheSnapshots, FileCacheLog, GenericMap, InstantiationMap, LiveCacheHandles, MethodMap,
    OverloadMap, PhysicalMap, ReservationLookup, ReservationTable, TAG_GENERIC, TAG_INSTANTIATION,
    TAG_METHOD, TAG_OVERLOAD, TAG_PHYSICAL, TAG_TEMPLATE, TemplateMap, digest_bucket, digest_flat,
};

pub(super) struct WorkerOverlay {
    pub(super) generic: FxHashMap<DeclarationResolutionKey, Vec<(usize, GenericInstantiationCacheEntry)>>,
    pub(super) instantiations: FxHashMap<DeclarationResolutionKey, Vec<(usize, InstantiationCacheEntry)>>,
    pub(super) physical: FxHashMap<InterfaceInstantiationKey, (usize, Arc<Type>)>,
    pub(super) templates: FxHashMap<StableInterfaceDeclarationId, (usize, Arc<InterfaceDeclarationTemplate>)>,
    pub(super) methods: FxHashMap<InterfaceMemberInstantiationKey, (usize, FunctionType)>,
    pub(super) overloads: FxHashMap<InterfaceOverloadInstantiationKey, (usize, FunctionType)>,
    /// Instantiation digests this attempt has already deferred once. A deferred
    /// key must defer at most once per attempt: the nominal reference the peel
    /// returns re-enters resolution when it is later forced (`Type::peeled` is
    /// `reference.resolve().peeled()`), and a second deferral would hand back
    /// another deferring nominal, so peeling would recurse forever. On the
    /// second lookup the key resolves as a normal miss (expands to a concrete
    /// type), terminating the peel. The attempt is discarded and requeued
    /// regardless, so this only bounds work, never changes the committed result.
    pub(super) deferred_once: FxHashSet<u64>,
    pub(super) current: FileCacheLog,
    pub(super) current_active: bool,
    pub(super) finished: Vec<FileCacheLog>,
}

impl WorkerOverlay {
    pub(super) fn new() -> Self {
        Self {
            generic: FxHashMap::default(),
            instantiations: FxHashMap::default(),
            physical: FxHashMap::default(),
            templates: FxHashMap::default(),
            methods: FxHashMap::default(),
            overloads: FxHashMap::default(),
            deferred_once: FxHashSet::default(),
            current: FileCacheLog::default(),
            current_active: false,
            finished: Vec::new(),
        }
    }

    pub(super) fn record_dep(&mut self, inserting_file: usize) {
        if inserting_file != self.current.file_index {
            self.current.overlay_deps.insert(inserting_file);
        }
    }
}

/// The committed cache state a session reads beneath its overlay. Workers use
/// an immutable [`CacheSnapshots`] taken at fan-out (lock-free, shared by all
/// workers); coordinator rechecks read the live maps directly — the commit
/// pass is single-threaded, so the live maps are exactly the committed state
/// and cloning a fresh snapshot per recheck would dominate commit time.
pub(super) enum BaseView {
    Snapshot(Arc<CacheSnapshots>),
    Live,
}

/// Measurement-only deferral context (Stage 2). When a replay session reads the
/// live committed store and misses a key, it consults the reservation table for
/// this position: a `Deferred` result means an earlier not-yet-committed
/// position would have published the key, so serial checking would have hit it
/// and the replay is about to over-recurse. This context only *counts* those
/// events (behind `SURGE_DEFER_MEASURE`); it never changes resolution, so output
/// stays byte-identical. It quantifies the deferral opportunity before any
/// abort-and-requeue mechanism is built.
pub(super) struct DeferralContext {
    pub(super) table: Arc<std::sync::RwLock<ReservationTable>>,
    pub(super) position: usize,
    pub(super) stats: Arc<DeferralStats>,
    /// Latest (largest) blocking publisher this attempt deferred to, or -1 if it
    /// never deferred. The requeue waits on the latest so the re-run reads the
    /// most-committed view; `-1` means the replay ran to completion (valid).
    pub(super) max_deferred: std::sync::atomic::AtomicI64,
}

impl DeferralContext {
    /// Queries the reservation table for this position, records the deferral in
    /// the stats and the attempt's `max_deferred`, and returns the blocking
    /// publisher if the key is owned by an earlier not-yet-committed position.
    pub(super) fn check(
        &self,
        query: impl FnOnce(&ReservationTable, usize) -> ReservationLookup,
    ) -> Option<usize> {
        use std::sync::atomic::Ordering::Relaxed;
        self.stats.queried.fetch_add(1, Relaxed);
        if let Ok(table) = self.table.read()
            && let ReservationLookup::Deferred { publisher, .. } = query(&table, self.position)
        {
            self.stats.deferred.fetch_add(1, Relaxed);
            self.max_deferred.fetch_max(publisher as i64, Relaxed);
            return Some(publisher);
        }
        None
    }
}

/// Outcome of a deferral-aware instantiation probe. `Deferred` is control flow —
/// never a `Type` — converted to a nominal reference only at the lazy-peel
/// boundary; every other caller treats it as a miss.
pub(crate) enum InstantiationProbe {
    Hit(InstantiationCacheEntry),
    Miss,
    Deferred,
}

/// Atomic counters shared across the replay pool for the deferral measurement.
#[derive(Default)]
pub(crate) struct DeferralStats {
    pub(crate) queried: std::sync::atomic::AtomicU64,
    pub(crate) deferred: std::sync::atomic::AtomicU64,
}

/// One worker's speculative view of the six program caches.
pub(crate) struct CheckSession {
    pub(super) live: LiveCacheHandles,
    pub(super) base: BaseView,
    pub(super) state: Mutex<WorkerOverlay>,
    pub(super) defer: Option<DeferralContext>,
}

impl CheckSession {
    pub(crate) fn new(live: LiveCacheHandles, base: Arc<CacheSnapshots>) -> Self {
        Self {
            live,
            base: BaseView::Snapshot(base),
            state: Mutex::new(WorkerOverlay::new()),
            defer: None,
        }
    }

    /// A session whose base reads go straight to the live maps. Only sound
    /// while nothing else writes them — i.e. during the single-threaded commit
    /// pass's rechecks.
    pub(crate) fn new_live_reading(live: LiveCacheHandles) -> Self {
        Self {
            live,
            base: BaseView::Live,
            state: Mutex::new(WorkerOverlay::new()),
            defer: None,
        }
    }

    /// A live-reading session that additionally records (measurement only) how
    /// often a miss would defer to an earlier pending publisher at `position`.
    pub(crate) fn new_live_reading_deferring(
        live: LiveCacheHandles,
        table: Arc<std::sync::RwLock<ReservationTable>>,
        position: usize,
        stats: Arc<DeferralStats>,
    ) -> Self {
        Self {
            live,
            base: BaseView::Live,
            state: Mutex::new(WorkerOverlay::new()),
            defer: Some(DeferralContext {
                table,
                position,
                stats,
                max_deferred: std::sync::atomic::AtomicI64::new(-1),
            }),
        }
    }

    /// The blocking publisher this replay deferred to (the latest), or `None` if
    /// it ran to completion without deferring. Used by the replay pipeline to
    /// requeue a deferred attempt once that publisher commits.
    pub(crate) fn deferred_until(&self) -> Option<usize> {
        self.defer.as_ref().and_then(|defer| {
            let value = defer
                .max_deferred
                .load(std::sync::atomic::Ordering::Relaxed);
            (value >= 0).then_some(value as usize)
        })
    }

    pub(super) fn base_generic_hit(
        &self,
        key: &DeclarationResolutionKey,
        arguments: &[Type],
    ) -> Option<(Type, bool)> {
        let digest = crate::context::arguments_digest(arguments);
        let find = |bucket: &[GenericInstantiationCacheEntry]| {
            bucket
                .iter()
                .find(|entry| entry.matches(digest, arguments))
                .map(|entry| (entry.ty.clone(), entry.had_error))
        };
        match &self.base {
            BaseView::Snapshot(base) => base.generic.get(key).and_then(|bucket| find(bucket)),
            BaseView::Live => self
                .live
                .generic
                .lock()
                .ok()
                .and_then(|cache| cache.get(key).and_then(|bucket| find(bucket))),
        }
    }

    /// Returns whether the base bucket already holds `arguments` and the
    /// bucket's current length (for the insertion cap).
    pub(super) fn base_generic_probe(
        &self,
        key: &DeclarationResolutionKey,
        arguments: &[Type],
    ) -> (bool, usize) {
        let digest = crate::context::arguments_digest(arguments);
        let probe = |bucket: &[GenericInstantiationCacheEntry]| {
            (
                bucket.iter().any(|entry| entry.matches(digest, arguments)),
                bucket.len(),
            )
        };
        match &self.base {
            BaseView::Snapshot(base) => base.generic.get(key).map_or((false, 0), |b| probe(b)),
            BaseView::Live => self.live.generic.lock().ok().map_or((false, 0), |cache| {
                cache.get(key).map_or((false, 0), |b| probe(b))
            }),
        }
    }

    pub(super) fn base_instantiation_entry(
        &self,
        key: &DeclarationResolutionKey,
        arguments: &[Type],
    ) -> Option<InstantiationCacheEntry> {
        let digest = crate::context::arguments_digest(arguments);
        let find = |bucket: &[InstantiationCacheEntry]| {
            bucket
                .iter()
                .find(|entry| entry.matches(digest, arguments))
                .cloned()
        };
        match &self.base {
            BaseView::Snapshot(base) => base.instantiations.get(key).and_then(|b| find(b)),
            BaseView::Live => self
                .live
                .instantiations
                .lock()
                .ok()
                .and_then(|cache| cache.get(key).and_then(|b| find(b))),
        }
    }

    pub(super) fn base_instantiation_len(&self, key: &DeclarationResolutionKey) -> usize {
        match &self.base {
            BaseView::Snapshot(base) => base.instantiations.get(key).map_or(0, Vec::len),
            BaseView::Live => self
                .live
                .instantiations
                .lock()
                .ok()
                .map_or(0, |cache| cache.get(key).map_or(0, Vec::len)),
        }
    }

    pub(super) fn base_physical(&self, key: &InterfaceInstantiationKey) -> Option<Arc<Type>> {
        match &self.base {
            BaseView::Snapshot(base) => base.physical.get(key).cloned(),
            BaseView::Live => self
                .live
                .physical
                .lock()
                .ok()
                .and_then(|cache| cache.get(key).cloned()),
        }
    }

    pub(super) fn base_template(
        &self,
        key: &StableInterfaceDeclarationId,
    ) -> Option<Arc<InterfaceDeclarationTemplate>> {
        match &self.base {
            BaseView::Snapshot(base) => base.templates.get(key).cloned(),
            BaseView::Live => self
                .live
                .templates
                .lock()
                .ok()
                .and_then(|cache| cache.get(key).cloned()),
        }
    }

    pub(super) fn base_method(&self, key: &InterfaceMemberInstantiationKey) -> Option<FunctionType> {
        match &self.base {
            BaseView::Snapshot(base) => base.methods.get(key).cloned(),
            BaseView::Live => self
                .live
                .methods
                .lock()
                .ok()
                .and_then(|cache| cache.get(key).cloned()),
        }
    }

    pub(super) fn base_overload(&self, key: &InterfaceOverloadInstantiationKey) -> Option<FunctionType> {
        match &self.base {
            BaseView::Snapshot(base) => base.overloads.get(key).cloned(),
            BaseView::Live => self
                .live
                .overloads
                .lock()
                .ok()
                .and_then(|cache| cache.get(key).cloned()),
        }
    }

    /// Starts recording a new file's observations. Files checked by one worker
    /// arrive in ascending file order (the dispatch counter is monotonic), so
    /// the worker overlay only ever contains entries from files earlier in
    /// serial order than the current one.
    pub(crate) fn begin_file(&self, file_index: usize) {
        let mut state = self.state.lock().expect("check session poisoned");
        if state.current_active {
            let finished = std::mem::take(&mut state.current);
            state.finished.push(finished);
        }
        state.current = FileCacheLog {
            file_index,
            ..FileCacheLog::default()
        };
        state.current_active = true;
    }

    /// Flushes the in-progress file log and returns every file log this worker
    /// produced, in the worker's (ascending) check order.
    pub(crate) fn take_file_logs(&self) -> Vec<FileCacheLog> {
        let mut state = self.state.lock().expect("check session poisoned");
        if state.current_active {
            let finished = std::mem::take(&mut state.current);
            state.finished.push(finished);
            state.current_active = false;
        }
        std::mem::take(&mut state.finished)
    }

    pub(crate) fn owns_generic(&self, handle: &Arc<Mutex<GenericMap>>) -> bool {
        Arc::ptr_eq(handle, &self.live.generic)
    }

    pub(crate) fn owns_instantiations(&self, handle: &Arc<Mutex<InstantiationMap>>) -> bool {
        Arc::ptr_eq(handle, &self.live.instantiations)
    }

    pub(crate) fn owns_physical(&self, handle: &Arc<Mutex<PhysicalMap>>) -> bool {
        Arc::ptr_eq(handle, &self.live.physical)
    }

    pub(crate) fn owns_templates(&self, handle: &Arc<Mutex<TemplateMap>>) -> bool {
        Arc::ptr_eq(handle, &self.live.templates)
    }

    pub(crate) fn owns_methods(&self, handle: &Arc<Mutex<MethodMap>>) -> bool {
        Arc::ptr_eq(handle, &self.live.methods)
    }

    pub(crate) fn owns_overloads(&self, handle: &Arc<Mutex<OverloadMap>>) -> bool {
        Arc::ptr_eq(handle, &self.live.overloads)
    }

    /// Mirrors `get_persistent_generic_resolution`: snapshot bucket first (the
    /// entries serial checking would have started with, in their original
    /// order), then the worker overlay (this worker's own earlier appends).
    pub(crate) fn generic_lookup(
        &self,
        key: &DeclarationResolutionKey,
        arguments: &[Type],
    ) -> Option<(Type, bool)> {
        if let Some(hit) = self.base_generic_hit(key, arguments) {
            return Some(hit);
        }
        let digest = crate::context::arguments_digest(arguments);
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, entry)) = state.generic.get(key).and_then(|bucket| {
            bucket
                .iter()
                .find(|(_, entry)| entry.matches(digest, arguments))
                .cloned()
        }) {
            state.record_dep(file);
            return Some((entry.ty.clone(), entry.had_error));
        }
        let digest = digest_bucket(TAG_GENERIC, key, arguments);
        state.current.misses.insert(digest);
        if let Some(defer) = &self.defer {
            defer.check(|table, k| table.query_generic(key, arguments, k));
        }
        None
    }

    /// Mirrors `cache_persistent_generic_resolution` (probe, cap, insert).
    pub(crate) fn generic_insert(
        &self,
        key: &DeclarationResolutionKey,
        arguments: Vec<Type>,
        ty: Type,
        had_error: bool,
        cap: usize,
    ) {
        let (base_exists, base_len) = self.base_generic_probe(key, &arguments);
        if base_exists {
            return;
        }
        let arguments_digest = crate::context::arguments_digest(&arguments);
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some(bucket) = state.generic.get(key)
            && bucket.iter().any(|(_, entry)| entry.matches(arguments_digest, &arguments))
        {
            return;
        }
        let digest = digest_bucket(TAG_GENERIC, key, &arguments);
        state.current.misses.insert(digest);
        let overlay_len = state.generic.get(key).map_or(0, Vec::len);
        if base_len + overlay_len >= cap {
            crate::program::record_program_counter(|c| c.generic_type_cache_capped_count += 1);
            return;
        }
        crate::program::record_program_counter(|c| c.generic_type_cache_insert_count += 1);
        let entry = GenericInstantiationCacheEntry {
            arguments,
            arguments_digest,
            ty,
            had_error,
        };
        let file_index = state.current.file_index;
        state
            .current
            .generic_inserts
            .push((key.clone(), entry.clone(), digest));
        state
            .generic
            .entry(key.clone())
            .or_default()
            .push((file_index, entry));
    }

    /// Mirrors `intern_instantiation` (hit returns the shared expansion, miss
    /// interns under the bucket cap).
    pub(crate) fn instantiation_intern(
        &self,
        key: &DeclarationResolutionKey,
        arguments: &[Type],
        structural: Type,
        cap: usize,
    ) -> Arc<Type> {
        if let Some(entry) = self.base_instantiation_entry(key, arguments) {
            crate::program::record_program_counter(|c| c.instantiation_intern_hit_count += 1);
            return entry.resolved;
        }
        let base_len = self.base_instantiation_len(key);
        let arguments_digest = crate::context::arguments_digest(arguments);
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, resolved)) = state.instantiations.get(key).and_then(|bucket| {
            bucket
                .iter()
                .find(|(_, entry)| entry.matches(arguments_digest, arguments))
                .map(|(file, entry)| (*file, entry.resolved.clone()))
        }) {
            state.record_dep(file);
            crate::program::record_program_counter(|c| c.instantiation_intern_hit_count += 1);
            return resolved;
        }
        let digest = digest_bucket(TAG_INSTANTIATION, key, arguments);
        state.current.misses.insert(digest);
        let resolved = Arc::new(structural);
        let overlay_len = state.instantiations.get(key).map_or(0, Vec::len);
        if base_len + overlay_len < cap {
            crate::program::record_program_counter(|c| c.instantiation_intern_insert_count += 1);
            let entry = InstantiationCacheEntry {
                arguments: arguments.to_vec(),
                arguments_digest,
                resolved: resolved.clone(),
            };
            let file_index = state.current.file_index;
            state
                .current
                .instantiation_inserts
                .push((key.clone(), entry.clone(), digest));
            state
                .instantiations
                .entry(key.clone())
                .or_default()
                .push((file_index, entry));
        } else {
            crate::program::record_program_counter(|c| c.instantiation_intern_capped_count += 1);
        }
        resolved
    }

    /// Mirrors `lookup_instantiation`.
    pub(crate) fn instantiation_lookup(
        &self,
        key: &DeclarationResolutionKey,
        arguments: &[Type],
    ) -> Option<InstantiationCacheEntry> {
        match self.instantiation_probe(key, arguments) {
            InstantiationProbe::Hit(entry) => Some(entry),
            InstantiationProbe::Miss | InstantiationProbe::Deferred => None,
        }
    }

    /// Deferral-aware instantiation lookup used by the lazy peel: a miss whose
    /// key is owned by an earlier not-yet-committed publisher returns `Deferred`
    /// (at most once per key per attempt — see `WorkerOverlay::deferred_once`)
    /// instead of `Miss`, so the peel can return the nominal form rather than
    /// over-recursing into a declaration the earlier position will publish.
    pub(crate) fn instantiation_probe(
        &self,
        key: &DeclarationResolutionKey,
        arguments: &[Type],
    ) -> InstantiationProbe {
        if let Some(entry) = self.base_instantiation_entry(key, arguments) {
            return InstantiationProbe::Hit(entry);
        }
        let digest = crate::context::arguments_digest(arguments);
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, entry)) = state.instantiations.get(key).and_then(|bucket| {
            bucket
                .iter()
                .find(|(_, entry)| entry.matches(digest, arguments))
                .cloned()
        }) {
            state.record_dep(file);
            return InstantiationProbe::Hit(entry);
        }
        let digest = digest_bucket(TAG_INSTANTIATION, key, arguments);
        state.current.misses.insert(digest);
        if let Some(defer) = &self.defer
            && !state.deferred_once.contains(&digest)
            && defer
                .check(|table, k| table.query_instantiation(key, arguments, k))
                .is_some()
        {
            state.deferred_once.insert(digest);
            return InstantiationProbe::Deferred;
        }
        InstantiationProbe::Miss
    }

    pub(crate) fn physical_lookup(&self, key: &InterfaceInstantiationKey) -> Option<Arc<Type>> {
        if let Some(resolved) = self.base_physical(key) {
            return Some(resolved);
        }
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, resolved)) = state.physical.get(key).cloned() {
            state.record_dep(file);
            return Some(resolved);
        }
        let digest = digest_flat(TAG_PHYSICAL, key);
        state.current.misses.insert(digest);
        if let Some(defer) = &self.defer {
            defer.check(|table, k| table.query_physical(key, k));
        }
        None
    }

    pub(crate) fn physical_intern(
        &self,
        key: InterfaceInstantiationKey,
        resolved: Type,
    ) -> Arc<Type> {
        if let Some(existing) = self.base_physical(&key) {
            crate::program::record_program_counter(|c| {
                c.physical_interface_cache_racing_insert_count += 1
            });
            return existing;
        }
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, existing)) = state.physical.get(&key).cloned() {
            state.record_dep(file);
            crate::program::record_program_counter(|c| {
                c.physical_interface_cache_racing_insert_count += 1
            });
            return existing;
        }
        let digest = digest_flat(TAG_PHYSICAL, &key);
        state.current.misses.insert(digest);
        let resolved = Arc::new(resolved);
        crate::program::record_program_counter(|c| c.physical_interface_cache_insert_count += 1);
        let file_index = state.current.file_index;
        state
            .current
            .physical_inserts
            .push((key.clone(), resolved.clone(), digest));
        state.physical.insert(key, (file_index, resolved.clone()));
        resolved
    }

    pub(crate) fn template_lookup(
        &self,
        key: &StableInterfaceDeclarationId,
    ) -> Option<Arc<InterfaceDeclarationTemplate>> {
        if let Some(template) = self.base_template(key) {
            return Some(template);
        }
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, template)) = state.templates.get(key).cloned() {
            state.record_dep(file);
            return Some(template);
        }
        let digest = digest_flat(TAG_TEMPLATE, key);
        state.current.misses.insert(digest);
        if let Some(defer) = &self.defer {
            defer.check(|table, k| table.query_template(key, k));
        }
        None
    }

    pub(crate) fn template_intern(
        &self,
        key: StableInterfaceDeclarationId,
        template: Arc<InterfaceDeclarationTemplate>,
        retained_bytes: u64,
    ) -> Arc<InterfaceDeclarationTemplate> {
        if let Some(existing) = self.base_template(&key) {
            return existing;
        }
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, existing)) = state.templates.get(&key).cloned() {
            state.record_dep(file);
            return existing;
        }
        let digest = digest_flat(TAG_TEMPLATE, &key);
        state.current.misses.insert(digest);
        crate::program::record_program_counter(|c| {
            c.interface_template_insert_count += 1;
            c.interface_template_retained_bytes += retained_bytes;
        });
        let file_index = state.current.file_index;
        state
            .current
            .template_inserts
            .push((key.clone(), template.clone(), digest));
        state.templates.insert(key, (file_index, template.clone()));
        template
    }

    pub(crate) fn method_lookup(
        &self,
        key: &InterfaceMemberInstantiationKey,
    ) -> Option<FunctionType> {
        if let Some(function) = self.base_method(key) {
            return Some(function);
        }
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, function)) = state.methods.get(key).cloned() {
            state.record_dep(file);
            return Some(function);
        }
        let digest = digest_flat(TAG_METHOD, key);
        state.current.misses.insert(digest);
        if let Some(defer) = &self.defer {
            defer.check(|table, k| table.query_method(key, k));
        }
        None
    }

    pub(crate) fn method_intern(
        &self,
        key: InterfaceMemberInstantiationKey,
        function: FunctionType,
        key_bytes: u64,
        value_bytes: u64,
    ) -> FunctionType {
        if let Some(existing) = self.base_method(&key) {
            return existing;
        }
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, existing)) = state.methods.get(&key).cloned() {
            state.record_dep(file);
            return existing;
        }
        let digest = digest_flat(TAG_METHOD, &key);
        state.current.misses.insert(digest);
        crate::program::record_program_counter(|c| {
            c.interface_method_cache_insert_count += 1;
            c.interface_method_cache_key_bytes += key_bytes;
            c.interface_method_cache_value_shallow_bytes += value_bytes;
        });
        let file_index = state.current.file_index;
        state
            .current
            .method_inserts
            .push((key.clone(), function.clone(), digest));
        state.methods.insert(key, (file_index, function.clone()));
        function
    }

    pub(crate) fn overload_lookup(
        &self,
        key: &InterfaceOverloadInstantiationKey,
    ) -> Option<FunctionType> {
        if let Some(function) = self.base_overload(key) {
            return Some(function);
        }
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, function)) = state.overloads.get(key).cloned() {
            state.record_dep(file);
            return Some(function);
        }
        let digest = digest_flat(TAG_OVERLOAD, key);
        state.current.misses.insert(digest);
        if let Some(defer) = &self.defer {
            defer.check(|table, k| table.query_overload(key, k));
        }
        None
    }

    pub(crate) fn overload_intern(
        &self,
        key: InterfaceOverloadInstantiationKey,
        function: FunctionType,
        key_bytes: u64,
        value_bytes: u64,
    ) -> FunctionType {
        if let Some(existing) = self.base_overload(&key) {
            return existing;
        }
        let mut state = self.state.lock().expect("check session poisoned");
        if let Some((file, existing)) = state.overloads.get(&key).cloned() {
            state.record_dep(file);
            return existing;
        }
        let digest = digest_flat(TAG_OVERLOAD, &key);
        state.current.misses.insert(digest);
        crate::program::record_program_counter(|c| {
            c.interface_overload_cache_insert_count += 1;
            c.interface_overload_cache_key_bytes += key_bytes;
            c.interface_overload_cache_value_shallow_bytes += value_bytes;
        });
        let file_index = state.current.file_index;
        state
            .current
            .overload_inserts
            .push((key.clone(), function.clone(), digest));
        state.overloads.insert(key, (file_index, function.clone()));
        function
    }
}

thread_local! {
    static ACTIVE_CHECK_SESSION: RefCell<Option<Arc<CheckSession>>> = const { RefCell::new(None) };
}

pub(crate) fn active_check_session() -> Option<Arc<CheckSession>> {
    ACTIVE_CHECK_SESSION.with(|slot| slot.borrow().clone())
}

/// Installs `session` as the current thread's speculative view for the duration
/// of `f`. Restores the previous session on exit (including unwinds), so a
/// worker panic cannot leak its session into unrelated work on a reused thread.
pub(crate) fn with_check_session<R>(session: Arc<CheckSession>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Arc<CheckSession>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE_CHECK_SESSION.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = ACTIVE_CHECK_SESSION.with(|slot| slot.borrow_mut().replace(session));
    let _restore = Restore(previous);
    f()
}
