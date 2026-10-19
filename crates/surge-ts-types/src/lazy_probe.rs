//! `SURGE_LAZY_PROBE=1`: counters that size reference-aware relations and lazy
//! member types (docs/perf/LAZY-MEMBERS-AND-REFERENCE-RELATIONS.md, Stage 0).
//! Off by default; every hook is one cached flag read.

use std::cell::{Cell, RefCell};
use std::sync::{Arc, OnceLock};

use crate::fx::{FxHashMap, FxHashSet};
use crate::{PropertyMap, Type};

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_LAZY_PROBE").is_some())
}

thread_local! {
    static COUNTS: RefCell<FxHashMap<&'static str, u64>> = RefCell::default();
    static RELATION_DEPTH: Cell<u32> = const { Cell::new(0) };
    // Pinned so an address is never reused for another map while it counts.
    static EXPANSION_MAPS: RefCell<FxHashMap<usize, Arc<PropertyMap>>> = RefCell::default();
    static ACCESS_READS: RefCell<FxHashSet<(usize, Arc<str>)>> = RefCell::default();
    static RELATION_READS: RefCell<FxHashSet<(usize, Arc<str>)>> = RefCell::default();
}

/// Stage 1 of the design: relations and checks read what a reference already
/// answers instead of peeling it. `SURGE_REF_RELATIONS=0` restores the
/// peel-first order for A/B measurement.
pub fn ref_relations_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("SURGE_REF_RELATIONS").as_deref() != Ok("0"))
}

pub fn count(name: &'static str) {
    if enabled() {
        COUNTS.with(|counts| *counts.borrow_mut().entry(name).or_default() += 1);
    }
}

pub fn count_if(condition: bool, name: &'static str) {
    if condition {
        count(name);
    }
}

/// Marks a relation query in progress, so a forced expansion or a member read
/// beneath it is attributed to relations.
pub struct RelationScope(bool);

impl RelationScope {
    pub fn enter() -> Self {
        if !enabled() {
            return Self(false);
        }
        RELATION_DEPTH.with(|depth| depth.set(depth.get() + 1));
        Self(true)
    }
}

impl Drop for RelationScope {
    fn drop(&mut self) {
        if self.0 {
            RELATION_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
}

pub fn in_relation() -> bool {
    enabled() && RELATION_DEPTH.with(Cell::get) > 0
}

/// Records the member map an interface expansion built.
pub fn register_expansion_map(map: &Arc<PropertyMap>) {
    if enabled() {
        EXPANSION_MAPS.with(|maps| {
            maps.borrow_mut()
                .entry(Arc::as_ptr(map) as usize)
                .or_insert_with(|| map.clone());
        });
    }
}

/// Records a read of `name` from `map` when `map` came from an expansion.
pub fn record_read(map: &Arc<PropertyMap>, name: &str) {
    if !enabled() {
        return;
    }
    let address = Arc::as_ptr(map) as usize;
    if !EXPANSION_MAPS.with(|maps| maps.borrow().contains_key(&address)) {
        return;
    }
    let reads = if in_relation() { &RELATION_READS } else { &ACCESS_READS };
    reads.with(|reads| {
        reads.borrow_mut().insert((address, Arc::from(name)));
    });
}

pub fn dump() {
    if !enabled() {
        return;
    }
    COUNTS.with(|counts| {
        let counts = counts.borrow();
        let mut rows: Vec<_> = counts.iter().collect();
        rows.sort();
        for (name, value) in rows {
            eprintln!("[lazy-probe] {name} {value}");
        }
    });
    #[derive(Default)]
    struct Tally {
        members: u64,
        access: u64,
        relation: u64,
        any: u64,
    }
    let (mut methods, mut properties) = (Tally::default(), Tally::default());
    let maps = EXPANSION_MAPS.with(|maps| maps.borrow().len());
    ACCESS_READS.with(|access| {
        RELATION_READS.with(|relation| {
            let (access, relation) = (access.borrow(), relation.borrow());
            EXPANSION_MAPS.with(|maps| {
                for (address, map) in maps.borrow().iter() {
                    for (name, property) in map.iter() {
                        let tally = if property.method || matches!(property.ty, Type::Function(_)) {
                            &mut methods
                        } else {
                            &mut properties
                        };
                        let key = (*address, name.clone());
                        let (read_access, read_relation) = (access.contains(&key), relation.contains(&key));
                        tally.members += 1;
                        tally.access += u64::from(read_access);
                        tally.relation += u64::from(read_relation);
                        tally.any += u64::from(read_access || read_relation);
                    }
                }
            });
        });
    });
    let percent = |part: u64, whole: u64| if whole == 0 { 0.0 } else { 100.0 * part as f64 / whole as f64 };
    eprintln!("[lazy-probe] expansion_maps {maps}");
    for (label, tally) in [("methods", &methods), ("properties", &properties)] {
        eprintln!(
            "[lazy-probe] {label}: materialized={} read_by_access={} ({:.1}%) read_by_relation={} ({:.1}%) read_any={} ({:.1}%)",
            tally.members,
            tally.access,
            percent(tally.access, tally.members),
            tally.relation,
            percent(tally.relation, tally.members),
            tally.any,
            percent(tally.any, tally.members),
        );
    }
}
