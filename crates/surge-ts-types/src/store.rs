use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError, Weak};

use crate::function::record_function_type_payload_alloc_count;
use crate::fx::{FxHashMap, FxHasher, PrehashedU64Map};
use crate::union::record_union_type_payload_alloc_count;
use crate::{FunctionType, FunctionTypePayload, PropertyMap, Type, UnionTypePayload};

static NEXT_STORE_OWNER: AtomicU32 = AtomicU32::new(1);
const STORE_SHARDS: usize = 64;

// Owners start at 1, so an id is never zero and `Option<Id>` costs no more
// than the id: these sit in every `ObjectType` and `FunctionType`.
macro_rules! store_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(std::num::NonZeroU64);

        impl $name {
            fn new(owner: u32, index: u32) -> Self {
                Self(
                    std::num::NonZeroU64::new((u64::from(owner) << 32) | u64::from(index))
                        .expect("store owners are nonzero"),
                )
            }

            pub fn belongs_to(self, store: &ProgramTypeStore) -> bool {
                (self.0.get() >> 32) as u32 == store.owner
            }
        }
    };
}

store_id!(TypeListId);
store_id!(FunctionTypeId);
store_id!(UnionTypeId);
store_id!(PropertyMapId);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProgramTypeStoreRetainedCensus {
    pub function_payloads: u64,
    pub parameter_lists: u64,
    pub parameter_list_elements: u64,
    pub union_payloads: u64,
    pub union_member_elements: u64,
    pub property_maps: u64,
    pub property_map_entries: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProgramTypeStoreStats {
    pub parameter_list_requests: u64,
    pub parameter_list_hits: u64,
    pub parameter_list_misses: u64,
    pub parameter_list_input_elements: u64,
    pub parameter_list_stored_elements: u64,
    pub parameter_list_elements_avoided: u64,
    pub function_requests: u64,
    pub function_hits: u64,
    pub function_misses: u64,
    pub function_fallbacks: u64,
    pub overload_merge_requests: u64,
    pub overload_merge_hits: u64,
    pub overload_merge_misses: u64,
    pub union_requests: u64,
    pub union_hits: u64,
    pub union_misses: u64,
    pub union_member_elements_avoided: u64,
    pub property_map_requests: u64,
    pub property_map_hits: u64,
    pub property_map_misses: u64,
    pub property_entries_avoided: u64,
    pub interner_lock_contentions: u64,
}

// Bucket entries hold `Weak` payload references so an interned payload lives
// exactly as long as its consumers: a canonical type produced by a transient
// pass (preliminary analysis, per-file inference) frees with that pass instead
// of accumulating in the store for the whole program. A `Weak` still keeps the
// payload's allocation, so dead entries must leave the store too: a bucket
// drops them whenever it is next scanned, and since most fingerprints are never
// interned again, a whole shard is swept once it has doubled since its last
// sweep (on tRPC, 9 in 10 function payloads were dead by the end of the check
// phase). IDs are monotonic and never reused, so a re-interned equal payload
// getting a fresh ID cannot collide with identity fast-paths that compared the
// dead one — no live value can hold a dead ID.
#[derive(Debug)]
struct ListEntry {
    id: TypeListId,
    value: Weak<[Type]>,
}

#[derive(Debug)]
struct FunctionEntry {
    id: FunctionTypeId,
    value: Weak<FunctionTypePayload>,
}

#[derive(Debug)]
struct UnionEntry {
    id: UnionTypeId,
    value: Weak<UnionTypePayload>,
}

#[derive(Debug)]
struct PropertyMapEntry {
    id: PropertyMapId,
    value: Weak<PropertyMap>,
}

#[derive(Debug, Default)]
struct StoreCounters {
    parameter_list_requests: AtomicU64,
    parameter_list_hits: AtomicU64,
    parameter_list_misses: AtomicU64,
    parameter_list_input_elements: AtomicU64,
    parameter_list_stored_elements: AtomicU64,
    parameter_list_elements_avoided: AtomicU64,
    function_requests: AtomicU64,
    function_hits: AtomicU64,
    function_misses: AtomicU64,
    function_fallbacks: AtomicU64,
    overload_merge_requests: AtomicU64,
    overload_merge_hits: AtomicU64,
    overload_merge_misses: AtomicU64,
    union_requests: AtomicU64,
    union_hits: AtomicU64,
    union_misses: AtomicU64,
    union_member_elements_avoided: AtomicU64,
    property_map_requests: AtomicU64,
    property_map_hits: AtomicU64,
    property_map_misses: AtomicU64,
    property_entries_avoided: AtomicU64,
    interner_lock_contentions: AtomicU64,
}

#[derive(Debug)]
pub struct ProgramTypeStore {
    owner: u32,
    /// The program's `strictNullChecks`, installed for the thread by
    /// [`with_program_type_store`] so type construction can read it.
    strict_null_checks: std::sync::atomic::AtomicBool,
    /// The program's `exactOptionalPropertyTypes`, installed alongside it.
    exact_optional_property_types: std::sync::atomic::AtomicBool,
    next_type_list: AtomicU32,
    next_function: AtomicU32,
    next_union: AtomicU32,
    next_property_map: AtomicU32,
    parameter_lists: [Mutex<PrehashedU64Map<Vec<ListEntry>>>; STORE_SHARDS],
    functions: [Mutex<PrehashedU64Map<Vec<FunctionEntry>>>; STORE_SHARDS],
    overload_merges: [Mutex<
        FxHashMap<(FunctionTypeId, FunctionTypeId), (FunctionTypeId, Weak<FunctionTypePayload>)>,
    >; STORE_SHARDS],
    unions: [Mutex<PrehashedU64Map<Vec<UnionEntry>>>; STORE_SHARDS],
    property_maps: [Mutex<PrehashedU64Map<Vec<PropertyMapEntry>>>; STORE_SHARDS],
    sweep_due: SweepThresholds,
    counters: StoreCounters,
}

/// Per shard, the length at which the shard is next swept of dead entries.
/// Each threshold is only read and written under its shard's lock.
#[derive(Debug)]
struct SweepThresholds {
    parameter_lists: [AtomicUsize; STORE_SHARDS],
    functions: [AtomicUsize; STORE_SHARDS],
    overload_merges: [AtomicUsize; STORE_SHARDS],
    unions: [AtomicUsize; STORE_SHARDS],
    property_maps: [AtomicUsize; STORE_SHARDS],
}

const MIN_SWEEP_LEN: usize = 1024;

impl Default for SweepThresholds {
    fn default() -> Self {
        let due = || std::array::from_fn(|_| AtomicUsize::new(MIN_SWEEP_LEN));
        Self {
            parameter_lists: due(),
            functions: due(),
            overload_merges: due(),
            unions: due(),
            property_maps: due(),
        }
    }
}

/// Drops the entries `live` rejects once `map` has doubled since its last
/// sweep, which keeps the sweeps amortized O(1) per insertion.
fn sweep_if_due<K, V, S>(
    map: &mut std::collections::HashMap<K, V, S>,
    due: &AtomicUsize,
    live: impl FnMut(&K, &mut V) -> bool,
) {
    if map.len() < due.load(Ordering::Relaxed) {
        return;
    }
    map.retain(live);
    due.store(
        map.len().saturating_mul(2).max(MIN_SWEEP_LEN),
        Ordering::Relaxed,
    );
}

fn retain_live_entries<E>(bucket: &mut Vec<E>, live: impl Fn(&E) -> bool) -> bool {
    bucket.retain(live);
    !bucket.is_empty()
}

impl ProgramTypeStore {
    pub fn new() -> Arc<Self> {
        let owner = NEXT_STORE_OWNER.fetch_add(1, Ordering::Relaxed);
        assert_ne!(owner, 0, "program type-store owner space exhausted");
        Arc::new(Self {
            owner,
            strict_null_checks: std::sync::atomic::AtomicBool::new(true),
            exact_optional_property_types: std::sync::atomic::AtomicBool::new(false),
            next_type_list: AtomicU32::new(1),
            next_function: AtomicU32::new(1),
            next_union: AtomicU32::new(1),
            next_property_map: AtomicU32::new(1),
            parameter_lists: std::array::from_fn(|_| Mutex::new(PrehashedU64Map::default())),
            functions: std::array::from_fn(|_| Mutex::new(PrehashedU64Map::default())),
            overload_merges: std::array::from_fn(|_| Mutex::new(FxHashMap::default())),
            unions: std::array::from_fn(|_| Mutex::new(PrehashedU64Map::default())),
            property_maps: std::array::from_fn(|_| Mutex::new(PrehashedU64Map::default())),
            sweep_due: SweepThresholds::default(),
            counters: StoreCounters::default(),
        })
    }

    pub(crate) fn intern_function(
        &self,
        parameters: Vec<Type>,
        return_type: Type,
        is_variadic: bool,
        required_parameter_count: usize,
    ) -> Result<(Arc<FunctionTypePayload>, FunctionTypeId), (Vec<Type>, Type)> {
        self.counters
            .function_requests
            .fetch_add(1, Ordering::Relaxed);
        let mut budget = FingerprintBudget::default();
        let Some(parameter_hash) = fingerprint_types(&parameters, &mut budget) else {
            return Err((parameters, return_type));
        };
        let Some(return_hash) = fingerprint_type(&return_type, &mut budget) else {
            return Err((parameters, return_type));
        };
        let (parameters, parameter_list_id) =
            self.intern_parameter_list(parameters, parameter_hash);
        let key = hash_key(&(
            parameter_hash,
            return_hash,
            is_variadic,
            required_parameter_count,
        ));
        let shard = shard_index(key);
        let mut functions = self.lock_shard(&self.functions[shard]);
        let bucket = functions.entry(key).or_default();
        let mut hit = None;
        bucket.retain(|entry| {
            if hit.is_some() {
                return true;
            }
            let Some(existing) = entry.value.upgrade() else {
                return false;
            };
            if canonical_type_lists_equal(existing.parameters.as_ref(), parameters.as_ref())
                && canonical_types_equal(&existing.return_type, &return_type)
                && existing.is_variadic == is_variadic
                && existing.required_parameter_count == required_parameter_count
            {
                hit = Some((existing, entry.id));
            }
            true
        });
        if let Some((existing, id)) = hit {
            self.counters.function_hits.fetch_add(1, Ordering::Relaxed);
            return Ok((existing, id));
        }

        let id = FunctionTypeId::new(
            self.owner,
            self.next_function.fetch_add(1, Ordering::Relaxed),
        );
        let payload = Arc::new(FunctionTypePayload {
            parameters,
            parameter_list_id: Some(parameter_list_id),
            return_type,
            is_variadic,
            required_parameter_count,
            name_memo: crate::name_memo::NameMemo::default(),
        });
        record_function_type_payload_alloc_count();
        bucket.push(FunctionEntry {
            id,
            value: Arc::downgrade(&payload),
        });
        sweep_if_due(&mut *functions, &self.sweep_due.functions[shard], |_, bucket| {
            retain_live_entries(bucket, |entry| entry.value.strong_count() > 0)
        });
        self.counters
            .function_misses
            .fetch_add(1, Ordering::Relaxed);
        Ok((payload, id))
    }

    pub(crate) fn record_function_fallback(&self) {
        self.counters
            .function_fallbacks
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn lookup_overload_merge(
        &self,
        left: FunctionTypeId,
        right: FunctionTypeId,
    ) -> Option<FunctionType> {
        self.counters
            .overload_merge_requests
            .fetch_add(1, Ordering::Relaxed);
        let key = (left, right);
        let hash = hash_key(&key);
        let mut merges = self.lock_shard(&self.overload_merges[shard_index(hash)]);
        let Some((id, payload)) = merges.get(&key) else {
            self.counters
                .overload_merge_misses
                .fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let id = *id;
        let Some(payload) = payload.upgrade() else {
            merges.remove(&key);
            self.counters
                .overload_merge_misses
                .fetch_add(1, Ordering::Relaxed);
            return None;
        };
        self.counters
            .overload_merge_hits
            .fetch_add(1, Ordering::Relaxed);
        Some(FunctionType::from_canonical_parts(payload, id))
    }

    pub fn record_overload_merge(
        &self,
        left: FunctionTypeId,
        right: FunctionTypeId,
        merged: FunctionType,
    ) -> FunctionType {
        let Some(id) = merged.id() else {
            return merged;
        };
        let key = (left, right);
        let shard = shard_index(hash_key(&key));
        let mut merges = self.lock_shard(&self.overload_merges[shard]);
        if let Some((existing_id, existing)) = merges.get(&key)
            && let Some(payload) = existing.upgrade()
        {
            return FunctionType::from_canonical_parts(payload, *existing_id);
        }
        merges.insert(key, (id, Arc::downgrade(&merged.payload)));
        sweep_if_due(&mut *merges, &self.sweep_due.overload_merges[shard], |_, (_, payload)| {
            payload.strong_count() > 0
        });
        merged
    }

    fn intern_parameter_list(&self, parameters: Vec<Type>, key: u64) -> (Arc<[Type]>, TypeListId) {
        self.counters
            .parameter_list_requests
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .parameter_list_input_elements
            .fetch_add(parameters.len() as u64, Ordering::Relaxed);
        let shard = shard_index(key);
        let mut lists = self.lock_shard(&self.parameter_lists[shard]);
        let bucket = lists.entry(key).or_default();
        let mut hit = None;
        bucket.retain(|entry| {
            if hit.is_some() {
                return true;
            }
            let Some(existing) = entry.value.upgrade() else {
                return false;
            };
            if canonical_type_lists_equal(existing.as_ref(), parameters.as_slice()) {
                hit = Some((existing, entry.id));
            }
            true
        });
        if let Some((existing, id)) = hit {
            self.counters
                .parameter_list_hits
                .fetch_add(1, Ordering::Relaxed);
            self.counters
                .parameter_list_elements_avoided
                .fetch_add(parameters.len() as u64, Ordering::Relaxed);
            return (existing, id);
        }

        let id = TypeListId::new(
            self.owner,
            self.next_type_list.fetch_add(1, Ordering::Relaxed),
        );
        let value: Arc<[Type]> = parameters.into();
        bucket.push(ListEntry {
            id,
            value: Arc::downgrade(&value),
        });
        sweep_if_due(&mut *lists, &self.sweep_due.parameter_lists[shard], |_, bucket| {
            retain_live_entries(bucket, |entry| entry.value.strong_count() > 0)
        });
        self.counters
            .parameter_list_misses
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .parameter_list_stored_elements
            .fetch_add(value.len() as u64, Ordering::Relaxed);
        (value, id)
    }

    pub(crate) fn intern_union(
        &self,
        types: Vec<Type>,
    ) -> Result<(Arc<UnionTypePayload>, UnionTypeId), Vec<Type>> {
        self.counters.union_requests.fetch_add(1, Ordering::Relaxed);
        let mut budget = FingerprintBudget::default();
        let Some(key) = fingerprint_types(&types, &mut budget) else {
            return Err(types);
        };
        let shard = shard_index(key);
        let mut unions = self.lock_shard(&self.unions[shard]);
        let bucket = unions.entry(key).or_default();
        let mut hit = None;
        bucket.retain(|entry| {
            if hit.is_some() {
                return true;
            }
            let Some(existing) = entry.value.upgrade() else {
                return false;
            };
            if canonical_type_lists_equal(existing.types.as_ref(), types.as_slice()) {
                hit = Some((existing, entry.id));
            }
            true
        });
        if let Some((existing, id)) = hit {
            self.counters.union_hits.fetch_add(1, Ordering::Relaxed);
            self.counters
                .union_member_elements_avoided
                .fetch_add(types.len() as u64, Ordering::Relaxed);
            return Ok((existing, id));
        }

        let id = UnionTypeId::new(self.owner, self.next_union.fetch_add(1, Ordering::Relaxed));
        let payload = Arc::new(UnionTypePayload {
            types: types.into(),
            list_id: None,
            name_memo: crate::name_memo::NameMemo::default(),
        });
        record_union_type_payload_alloc_count();
        bucket.push(UnionEntry {
            id,
            value: Arc::downgrade(&payload),
        });
        sweep_if_due(&mut *unions, &self.sweep_due.unions[shard], |_, bucket| {
            retain_live_entries(bucket, |entry| entry.value.strong_count() > 0)
        });
        self.counters.union_misses.fetch_add(1, Ordering::Relaxed);
        Ok((payload, id))
    }

    /// Borrowed-member interner probe for `union_type`: on a hit no member is
    /// cloned; on a miss (or an over-budget fingerprint) the caller falls back
    /// to the owned [`Self::intern_union`] path.
    pub(crate) fn intern_union_borrowed(
        &self,
        types: &[&Type],
    ) -> Option<(Arc<UnionTypePayload>, UnionTypeId)> {
        let mut budget = FingerprintBudget::default();
        let key = fingerprint_borrowed_types(types, &mut budget)?;
        let mut unions = self.lock_shard(&self.unions[shard_index(key)]);
        let bucket = unions.entry(key).or_default();
        let mut hit = None;
        bucket.retain(|entry| {
            if hit.is_some() {
                return true;
            }
            let Some(existing) = entry.value.upgrade() else {
                return false;
            };
            if existing.types.len() == types.len()
                && existing
                    .types
                    .iter()
                    .zip(types.iter())
                    .all(|(left, right)| canonical_types_equal(left, right))
            {
                hit = Some((existing, entry.id));
            }
            true
        });
        if let Some((existing, id)) = hit {
            self.counters.union_requests.fetch_add(1, Ordering::Relaxed);
            self.counters.union_hits.fetch_add(1, Ordering::Relaxed);
            self.counters
                .union_member_elements_avoided
                .fetch_add(types.len() as u64, Ordering::Relaxed);
            return Some((existing, id));
        }
        None
    }

    pub(crate) fn intern_property_map(
        &self,
        properties: PropertyMap,
    ) -> Result<(Arc<PropertyMap>, PropertyMapId), PropertyMap> {
        self.counters
            .property_map_requests
            .fetch_add(1, Ordering::Relaxed);
        let mut budget = FingerprintBudget::default();
        let mut hasher = FxHasher::default();
        properties.len().hash(&mut hasher);
        for (name, property) in &properties {
            name.hash(&mut hasher);
            property.optional.hash(&mut hasher);
            let Some(fingerprint) = fingerprint_property_type(&property.ty, &mut budget) else {
                return Err(properties);
            };
            fingerprint.hash(&mut hasher);
        }
        let key = hasher.finish();
        let shard = shard_index(key);
        let mut maps = self.lock_shard(&self.property_maps[shard]);
        let bucket = maps.entry(key).or_default();
        let mut hit = None;
        bucket.retain(|entry| {
            if hit.is_some() {
                return true;
            }
            let Some(existing) = entry.value.upgrade() else {
                return false;
            };
            if ordered_property_maps_equal(&existing, &properties) {
                hit = Some((existing, entry.id));
            }
            true
        });
        if let Some((existing, id)) = hit {
            self.counters
                .property_map_hits
                .fetch_add(1, Ordering::Relaxed);
            self.counters
                .property_entries_avoided
                .fetch_add(properties.len() as u64, Ordering::Relaxed);
            return Ok((existing, id));
        }

        let id = PropertyMapId::new(
            self.owner,
            self.next_property_map.fetch_add(1, Ordering::Relaxed),
        );
        let mut properties = properties;
        // A stored map lives as long as its consumers; the capacity it grew
        // through while its members were collected would live with it.
        properties.shrink_to_fit();
        let value = Arc::new(properties);
        bucket.push(PropertyMapEntry {
            id,
            value: Arc::downgrade(&value),
        });
        sweep_if_due(&mut *maps, &self.sweep_due.property_maps[shard], |_, bucket| {
            retain_live_entries(bucket, |entry| entry.value.strong_count() > 0)
        });
        self.counters
            .property_map_misses
            .fetch_add(1, Ordering::Relaxed);
        Ok((value, id))
    }

    /// Iterates every retained canonical payload (function parameter/return
    /// types, union members, property-map values) for census walks. Diagnostics
    /// only; takes each shard lock briefly.
    pub fn for_each_retained_type(&self, f: &mut dyn FnMut(&Type)) {
        for shard in &self.functions {
            for bucket in self.lock_shard(shard).values() {
                for entry in bucket {
                    let Some(value) = entry.value.upgrade() else {
                        continue;
                    };
                    for parameter in value.parameters.iter() {
                        f(parameter);
                    }
                    f(&value.return_type);
                }
            }
        }
        for shard in &self.unions {
            for bucket in self.lock_shard(shard).values() {
                for entry in bucket {
                    let Some(value) = entry.value.upgrade() else {
                        continue;
                    };
                    for member in value.types.iter() {
                        f(member);
                    }
                }
            }
        }
        for shard in &self.property_maps {
            for bucket in self.lock_shard(shard).values() {
                for entry in bucket {
                    let Some(value) = entry.value.upgrade() else {
                        continue;
                    };
                    for (_, property) in value.iter() {
                        f(&property.ty);
                    }
                }
            }
        }
    }

    /// Entry counts of the retained (still-live) canonical payloads, for census
    /// reporting.
    pub fn retained_census(&self) -> ProgramTypeStoreRetainedCensus {
        let mut census = ProgramTypeStoreRetainedCensus::default();
        for shard in &self.functions {
            for bucket in self.lock_shard(shard).values() {
                census.function_payloads +=
                    bucket.iter().filter(|e| e.value.strong_count() > 0).count() as u64;
            }
        }
        for shard in &self.parameter_lists {
            for bucket in self.lock_shard(shard).values() {
                for entry in bucket {
                    let Some(value) = entry.value.upgrade() else {
                        continue;
                    };
                    census.parameter_lists += 1;
                    census.parameter_list_elements += value.len() as u64;
                }
            }
        }
        for shard in &self.unions {
            for bucket in self.lock_shard(shard).values() {
                for entry in bucket {
                    let Some(value) = entry.value.upgrade() else {
                        continue;
                    };
                    census.union_payloads += 1;
                    census.union_member_elements += value.types.len() as u64;
                }
            }
        }
        for shard in &self.property_maps {
            for bucket in self.lock_shard(shard).values() {
                for entry in bucket {
                    let Some(value) = entry.value.upgrade() else {
                        continue;
                    };
                    census.property_maps += 1;
                    census.property_map_entries += value.len() as u64;
                }
            }
        }
        census
    }

    pub fn stats(&self) -> ProgramTypeStoreStats {
        let c = &self.counters;
        ProgramTypeStoreStats {
            parameter_list_requests: c.parameter_list_requests.load(Ordering::Relaxed),
            parameter_list_hits: c.parameter_list_hits.load(Ordering::Relaxed),
            parameter_list_misses: c.parameter_list_misses.load(Ordering::Relaxed),
            parameter_list_input_elements: c.parameter_list_input_elements.load(Ordering::Relaxed),
            parameter_list_stored_elements: c
                .parameter_list_stored_elements
                .load(Ordering::Relaxed),
            parameter_list_elements_avoided: c
                .parameter_list_elements_avoided
                .load(Ordering::Relaxed),
            function_requests: c.function_requests.load(Ordering::Relaxed),
            function_hits: c.function_hits.load(Ordering::Relaxed),
            function_misses: c.function_misses.load(Ordering::Relaxed),
            function_fallbacks: c.function_fallbacks.load(Ordering::Relaxed),
            overload_merge_requests: c.overload_merge_requests.load(Ordering::Relaxed),
            overload_merge_hits: c.overload_merge_hits.load(Ordering::Relaxed),
            overload_merge_misses: c.overload_merge_misses.load(Ordering::Relaxed),
            union_requests: c.union_requests.load(Ordering::Relaxed),
            union_hits: c.union_hits.load(Ordering::Relaxed),
            union_misses: c.union_misses.load(Ordering::Relaxed),
            union_member_elements_avoided: c.union_member_elements_avoided.load(Ordering::Relaxed),
            property_map_requests: c.property_map_requests.load(Ordering::Relaxed),
            property_map_hits: c.property_map_hits.load(Ordering::Relaxed),
            property_map_misses: c.property_map_misses.load(Ordering::Relaxed),
            property_entries_avoided: c.property_entries_avoided.load(Ordering::Relaxed),
            interner_lock_contentions: c.interner_lock_contentions.load(Ordering::Relaxed),
        }
    }

    fn lock_shard<'a, T>(&self, shard: &'a Mutex<T>) -> MutexGuard<'a, T> {
        match shard.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                self.counters
                    .interner_lock_contentions
                    .fetch_add(1, Ordering::Relaxed);
                shard.lock().unwrap_or_else(|error| error.into_inner())
            }
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        }
    }

    pub fn clear(&self) {
        for shard in &self.overload_merges {
            self.lock_shard(shard).clear();
        }
        for shard in &self.functions {
            self.lock_shard(shard).clear();
        }
        for shard in &self.parameter_lists {
            self.lock_shard(shard).clear();
        }
        for shard in &self.unions {
            self.lock_shard(shard).clear();
        }
        for shard in &self.property_maps {
            self.lock_shard(shard).clear();
        }
    }
}

fn ordered_property_maps_equal(left: &PropertyMap, right: &PropertyMap) -> bool {
    left.len() == right.len()
        && left.iter().zip(right.iter()).all(
            |((left_name, left_property), (right_name, right_property))| {
                left_name == right_name && left_property == right_property
            },
        )
}

#[derive(Debug)]
struct FingerprintBudget {
    remaining: usize,
    depth: usize,
}

impl Default for FingerprintBudget {
    fn default() -> Self {
        Self {
            remaining: 128,
            depth: 0,
        }
    }
}

fn fingerprint_borrowed_types(types: &[&Type], budget: &mut FingerprintBudget) -> Option<u64> {
    let mut hasher = FxHasher::default();
    types.len().hash(&mut hasher);
    for ty in types {
        fingerprint_type(ty, budget)?.hash(&mut hasher);
    }
    Some(hasher.finish())
}

fn fingerprint_types(types: &[Type], budget: &mut FingerprintBudget) -> Option<u64> {
    let mut hasher = FxHasher::default();
    types.len().hash(&mut hasher);
    for ty in types {
        fingerprint_type(ty, budget)?.hash(&mut hasher);
    }
    Some(hasher.finish())
}

fn fingerprint_type(ty: &Type, budget: &mut FingerprintBudget) -> Option<u64> {
    if budget.remaining == 0 || budget.depth >= 16 {
        return None;
    }
    budget.remaining -= 1;
    budget.depth += 1;
    let mut hasher = FxHasher::default();
    std::mem::discriminant(ty).hash(&mut hasher);
    match ty {
        Type::Unknown | Type::ErrorType | Type::TypeParameter(_) => return None,
        Type::StringLiteral(value) => value.hash(&mut hasher),
        Type::NumberLiteral(value) => value.value.hash(&mut hasher),
        Type::BooleanLiteral(value) => value.hash(&mut hasher),
        Type::Function(function) => function.payload_address().hash(&mut hasher),
        Type::Object(object) => {
            if object.keyed_indexes.is_some() {
                return None;
            }
            Arc::as_ptr(&object.properties).hash(&mut hasher);
            match object.string_index_type.as_deref() {
                Some(index) => fingerprint_type(index, budget)?.hash(&mut hasher),
                None => 0u8.hash(&mut hasher),
            }
            object
                .call_signature()
                .map(FunctionType::payload_address)
                .hash(&mut hasher);
            object
                .construct_signature()
                .map(FunctionType::payload_address)
                .hash(&mut hasher);
        }
        Type::Array(element) => fingerprint_type(element, budget)?.hash(&mut hasher),
        Type::Tuple(elements) => fingerprint_types(elements, budget)?.hash(&mut hasher),
        Type::OpenTuple(tuple) => {
            fingerprint_types(&tuple.leading, budget)?.hash(&mut hasher);
            fingerprint_type(&tuple.rest, budget)?.hash(&mut hasher);
            fingerprint_types(&tuple.trailing, budget)?.hash(&mut hasher);
        }
        Type::Union(union) => fingerprint_types(union.types(), budget)?.hash(&mut hasher),
        Type::Reference(reference) => {
            if reference.retains_resolution_context()
                || !reference.supports_program_canonicalization()
            {
                return None;
            }
            reference.id.hash(&mut hasher);
            reference
                .program_canonicalization_discriminator()
                .hash(&mut hasher);
            fingerprint_types(&reference.arguments, budget)?.hash(&mut hasher);
        }
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Undefined
        | Type::Null
        | Type::Void
        | Type::Any
        | Type::GenuineUnknown
        | Type::ErrorType
        | Type::Never => {}
    }
    budget.depth -= 1;
    Some(hasher.finish())
}

fn canonical_type_lists_equal(left: &[Type], right: &[Type]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| canonical_types_equal(left, right))
}

fn canonical_types_equal(left: &Type, right: &Type) -> bool {
    match (left, right) {
        // Display fields participate in canonical identity: an intern hit
        // substitutes the stored payload wholesale, so display-differing
        // variants of structurally-equal content must intern separately or
        // rendered diagnostic text becomes a function of intern order — which
        // is thread-schedule-dependent under parallel analysis.
        (Type::Reference(left), Type::Reference(right)) => {
            left.id == right.id
                && left.display == right.display
                && left.program_canonicalization_discriminator()
                    == right.program_canonicalization_discriminator()
                && canonical_type_lists_equal(&left.arguments, &right.arguments)
        }
        (Type::Array(left), Type::Array(right)) => canonical_types_equal(left, right),
        (Type::Tuple(left), Type::Tuple(right)) => canonical_type_lists_equal(left, right),
        (Type::Union(left), Type::Union(right)) => {
            canonical_type_lists_equal(left.types(), right.types())
        }
        (Type::Function(left), Type::Function(right)) => {
            left.payload_address() == right.payload_address() && left.this_type() == right.this_type()
        }
        (Type::Object(left), Type::Object(right)) => {
            left.alias_name == right.alias_name
                && left.alias_id == right.alias_id
                && Arc::ptr_eq(&left.properties, &right.properties)
                && left.string_index_readonly == right.string_index_readonly
                && left.number_index_readonly == right.number_index_readonly
                && left.keyed_indexes == right.keyed_indexes
                // `object` is not `{}`, nor an intersection or an open shape
                // the plain object it resembles: a hit on one for the other
                // made a narrowing's `{}` read as `object` once some file had
                // interned that first.
                && left.non_primitive == right.non_primitive
                && left.is_intersection == right.is_intersection
                && left.synthetic_open_index == right.synthetic_open_index
                && left.without_inferable_index == right.without_inferable_index
                && match (
                    left.intersection_operands.as_deref(),
                    right.intersection_operands.as_deref(),
                ) {
                    (Some(left), Some(right)) => canonical_type_lists_equal(left, right),
                    (None, None) => true,
                    _ => false,
                }
                && match (
                    left.number_index_type.as_deref(),
                    right.number_index_type.as_deref(),
                ) {
                    (Some(left), Some(right)) => canonical_types_equal(left, right),
                    (None, None) => true,
                    _ => false,
                }
                && match (
                    left.string_index_type.as_deref(),
                    right.string_index_type.as_deref(),
                ) {
                    (Some(left), Some(right)) => canonical_types_equal(left, right),
                    (None, None) => true,
                    _ => false,
                }
                && left
                    .call_signature()
                    .map(|function| function.payload_address())
                    == right
                        .call_signature()
                        .map(|function| function.payload_address())
                && left
                    .construct_signature()
                    .map(|function| function.payload_address())
                    == right
                        .construct_signature()
                        .map(|function| function.payload_address())
        }
        _ => left == right,
    }
}

fn fingerprint_property_type(ty: &Type, budget: &mut FingerprintBudget) -> Option<u64> {
    if matches!(
        ty,
        Type::Unknown | Type::TypeParameter(_) | Type::Function(_) | Type::Object(_) | Type::Reference(_)
    ) {
        return None;
    }
    if budget.remaining == 0 || budget.depth >= 16 {
        return None;
    }
    budget.remaining -= 1;
    budget.depth += 1;
    let mut hasher = FxHasher::default();
    std::mem::discriminant(ty).hash(&mut hasher);
    match ty {
        Type::StringLiteral(value) => value.hash(&mut hasher),
        Type::NumberLiteral(value) => value.value.hash(&mut hasher),
        Type::BooleanLiteral(value) => value.hash(&mut hasher),
        Type::Array(element) => fingerprint_property_type(element, budget)?.hash(&mut hasher),
        Type::Tuple(elements) => {
            for element in elements {
                fingerprint_property_type(element, budget)?.hash(&mut hasher);
            }
        }
        Type::OpenTuple(tuple) => {
            for element in tuple.leading.iter().chain(tuple.trailing.iter()) {
                fingerprint_property_type(element, budget)?.hash(&mut hasher);
            }
            fingerprint_property_type(&tuple.rest, budget)?.hash(&mut hasher);
        }
        Type::Union(union) => {
            for member in union.types() {
                fingerprint_property_type(member, budget)?.hash(&mut hasher);
            }
        }
        Type::String
        | Type::Number
        | Type::Boolean
        | Type::BigInt
        | Type::Symbol
        | Type::Undefined
        | Type::Null
        | Type::Void
        | Type::Any
        | Type::GenuineUnknown
        | Type::ErrorType
        | Type::Never => {}
        Type::Unknown
        | Type::TypeParameter(_)
        | Type::Function(_)
        | Type::Object(_)
        | Type::Reference(_) => unreachable!(),
    }
    budget.depth -= 1;
    Some(hasher.finish())
}

fn hash_key(value: &impl Hash) -> u64 {
    let mut hasher = FxHasher::default();
    value.hash(&mut hasher);
    hasher.finish()
}

fn shard_index(key: u64) -> usize {
    key as usize & (STORE_SHARDS - 1)
}

thread_local! {
    static ACTIVE_TYPE_STORE: RefCell<Option<Arc<ProgramTypeStore>>> = const { RefCell::new(None) };
    static STRICT_NULL_CHECKS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    static EXACT_OPTIONAL_PROPERTY_TYPES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl ProgramTypeStore {
    pub fn set_strict_null_checks(&self, strict: bool) {
        self.strict_null_checks.store(strict, Ordering::Relaxed);
    }

    pub fn set_exact_optional_property_types(&self, exact: bool) {
        self.exact_optional_property_types.store(exact, Ordering::Relaxed);
    }
}

pub fn with_program_type_store<R>(store: Arc<ProgramTypeStore>, f: impl FnOnce() -> R) -> R {
    let strict = store.strict_null_checks.load(Ordering::Relaxed);
    let exact = store.exact_optional_property_types.load(Ordering::Relaxed);
    let previous = ACTIVE_TYPE_STORE.with(|active| active.replace(Some(store)));
    struct Restore(Option<Arc<ProgramTypeStore>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE_TYPE_STORE.with(|active| {
                active.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(previous);
    with_exact_optional_property_types(exact, || with_strict_null_checks(strict, f))
}

/// Runs `f` with the thread's `strictNullChecks` set to `strict`.
pub fn with_strict_null_checks<R>(strict: bool, f: impl FnOnce() -> R) -> R {
    let previous = STRICT_NULL_CHECKS.with(|flag| flag.replace(strict));
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            STRICT_NULL_CHECKS.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(previous);
    f()
}

/// Whether the program being checked on this thread has `strictNullChecks`.
/// Without it `undefined` and `null` are in the domain of every type.
pub fn strict_null_checks() -> bool {
    STRICT_NULL_CHECKS.with(std::cell::Cell::get)
}

/// Runs `f` with the thread's `exactOptionalPropertyTypes` set to `exact`.
pub fn with_exact_optional_property_types<R>(exact: bool, f: impl FnOnce() -> R) -> R {
    let previous = EXACT_OPTIONAL_PROPERTY_TYPES.with(|flag| flag.replace(exact));
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            EXACT_OPTIONAL_PROPERTY_TYPES.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(previous);
    f()
}

/// Whether `exactOptionalPropertyTypes` is in effect on this thread: an
/// optional property then reads as its declared type plus tsc's *missing*
/// type, which `undefined` relates to but a written `undefined` does not
/// satisfy. Only meaningful under `strictNullChecks`, where optional
/// properties carry `undefined` at all.
pub fn exact_optional_property_types() -> bool {
    EXACT_OPTIONAL_PROPERTY_TYPES.with(std::cell::Cell::get) && strict_null_checks()
}

pub fn current_program_type_store() -> Option<Arc<ProgramTypeStore>> {
    ACTIVE_TYPE_STORE.with(|active| active.borrow().clone())
}

// These gates sit on every intern request (millions per run), so the env
// probes are read once per process rather than per call.
pub(crate) fn canonical_store_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_DISABLE_CANONICAL_TYPE_STORE").is_none())
}

pub(crate) fn canonical_function_store_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        canonical_store_enabled()
            && std::env::var_os("SURGE_DISABLE_CANONICAL_FUNCTION_STORE").is_none()
    })
}

pub(crate) fn canonical_union_store_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        canonical_store_enabled()
            && std::env::var_os("SURGE_DISABLE_CANONICAL_UNION_STORE").is_none()
    })
}

pub(crate) fn canonical_property_map_store_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        canonical_store_enabled()
            && std::env::var_os("SURGE_DISABLE_CANONICAL_PROPERTY_MAP_STORE").is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FunctionType, ObjectProperty, ObjectType, ResolveReference, TypeReference, UnionType,
    };

    struct EnvironmentResolver(u64);

    impl ResolveReference for EnvironmentResolver {
        fn resolve(&self) -> Type {
            Type::String
        }

        fn program_canonicalization_discriminator(&self) -> u64 {
            self.0
        }
    }

    fn environment_reference(discriminator: u64) -> Type {
        Type::Reference(TypeReference::new(
            "types.ts\0Value",
            "Value",
            Vec::new(),
            Arc::new(EnvironmentResolver(discriminator)),
        ))
    }

    #[test]
    fn function_payloads_and_parameter_lists_are_program_local() {
        let first_store = ProgramTypeStore::new();
        let (first, second) = with_program_type_store(first_store.clone(), || {
            (
                FunctionType::new(vec![Type::String], Type::Number, false, 1),
                FunctionType::new(vec![Type::String], Type::Number, false, 1),
            )
        });
        assert_eq!(first.id(), second.id());
        assert_eq!(first.parameter_list_id(), second.parameter_list_id());
        assert!(Arc::ptr_eq(&first.payload, &second.payload));

        let other = with_program_type_store(ProgramTypeStore::new(), || {
            FunctionType::new(vec![Type::String], Type::Number, false, 1)
        });
        assert_ne!(first.id(), other.id());
        assert!(!Arc::ptr_eq(&first.payload, &other.payload));
    }

    #[test]
    fn unknown_and_context_dependent_values_use_the_fallback_path() {
        let store = ProgramTypeStore::new();
        let (first, second) = with_program_type_store(store.clone(), || {
            (
                FunctionType::new(vec![Type::Unknown], Type::Number, false, 1),
                FunctionType::new(vec![Type::Unknown], Type::Number, false, 1),
            )
        });
        assert!(first.id().is_none());
        assert!(second.id().is_none());
        assert_eq!(store.stats().function_fallbacks, 2);
    }

    #[test]
    fn reference_environment_is_part_of_function_canonicalization() {
        with_program_type_store(ProgramTypeStore::new(), || {
            let first = FunctionType::new(vec![environment_reference(1)], Type::Number, false, 1);
            let same_environment =
                FunctionType::new(vec![environment_reference(1)], Type::Number, false, 1);
            let different_environment =
                FunctionType::new(vec![environment_reference(2)], Type::Number, false, 1);

            assert_eq!(first.id(), same_environment.id());
            assert_ne!(first.id(), different_environment.id());
        });
    }

    #[test]
    fn overload_merge_pairs_reuse_canonical_payloads() {
        let store = ProgramTypeStore::new();
        with_program_type_store(store.clone(), || {
            let left = FunctionType::new(vec![Type::String], Type::String, false, 1);
            let right = FunctionType::new(vec![Type::Number], Type::Number, false, 1);
            let left_id = left.id().unwrap();
            let right_id = right.id().unwrap();
            assert!(store.lookup_overload_merge(left_id, right_id).is_none());

            let merged = FunctionType::new(vec![Type::Any], Type::String, false, 1);
            let merged_id = merged.id();
            let _merged = store.record_overload_merge(left_id, right_id, merged);
            assert_eq!(
                store
                    .lookup_overload_merge(left_id, right_id)
                    .and_then(|function| function.id()),
                merged_id
            );
        });

        let stats = store.stats();
        assert_eq!(stats.overload_merge_requests, 2);
        assert_eq!(stats.overload_merge_hits, 1);
        assert_eq!(stats.overload_merge_misses, 1);
    }

    #[test]
    fn canonical_store_does_not_retain_dead_payloads() {
        let store = ProgramTypeStore::new();
        with_program_type_store(store.clone(), || {
            let function = FunctionType::new(vec![Type::String], Type::Number, false, 1);
            let first_id = function.id();
            let weak = Arc::downgrade(&function.payload);
            assert!(weak.upgrade().is_some());

            // Entries are weak: a payload lives exactly as long as its
            // consumers, not until store cleanup.
            drop(function);
            assert!(weak.upgrade().is_none());

            // A later equal intern re-creates the payload under a fresh,
            // never-reused id.
            let reinterned = FunctionType::new(vec![Type::String], Type::Number, false, 1);
            assert!(reinterned.id().is_some());
            assert_ne!(reinterned.id(), first_id);

            // Two live equal values still unify on one payload.
            let same = FunctionType::new(vec![Type::String], Type::Number, false, 1);
            assert_eq!(same.id(), reinterned.id());
            assert!(Arc::ptr_eq(&same.payload, &reinterned.payload));
        });
        store.clear();
    }

    #[test]
    fn unions_and_property_maps_preserve_order_while_sharing_payloads() {
        let store = ProgramTypeStore::new();
        with_program_type_store(store.clone(), || {
            let first = UnionType::new(vec![Type::String, Type::Number]);
            let second = UnionType::new(vec![Type::String, Type::Number]);
            assert_eq!(first.id(), second.id());

            let mut first_properties = PropertyMap::default();
            first_properties.insert("a".into(), ObjectProperty::required(Type::String));
            first_properties.insert("b".into(), ObjectProperty::required(Type::Number));
            let mut second_properties = PropertyMap::default();
            second_properties.insert("a".into(), ObjectProperty::required(Type::String));
            second_properties.insert("b".into(), ObjectProperty::required(Type::Number));
            let first_object = ObjectType::new(first_properties, None);
            let second_object = ObjectType::new(second_properties, None);
            assert!(Arc::ptr_eq(
                &first_object.properties,
                &second_object.properties
            ));
        });
    }
}
