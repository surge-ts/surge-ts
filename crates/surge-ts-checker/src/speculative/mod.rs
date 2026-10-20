//! Serial-equivalent speculative check sessions.
//!
//! The six program-wide resolution caches (`program_resolved_generic_types`,
//! `program_instantiations`, and the four `physical_interface_*` caches) are
//! order-visible: whether a file's resolution *hits* an entry seeded by an
//! earlier-checked file can change a rendered type display (nominal
//! `ReadonlyArray<Auth>` vs structural `Auth[]`), so racing parallel workers
//! against the shared maps changes output bytes between runs. Normalizing the
//! hit/miss display forms is forbidden — a prior attempt changed real
//! diagnostics (see `docs/perf/SPECULATIVE-TRANSACTIONAL-CHECKING.md` §3).
//!
//! This module instead makes parallel checking *serial-equivalent*: workers
//! never write the live caches. Each worker checks files against an immutable
//! snapshot of the caches taken at fan-out plus a private overlay of its own
//! insertions, and records per file which cache keys it observed *missing*.
//! After the workers finish, a single-threaded coordinator commits files in
//! serial file order: a file whose miss-set is disjoint from everything
//! published by earlier-ordered files behaved exactly as it would have under
//! serial checking, so its speculative result and cache insertions are
//! published as-is. A file that missed a key an earlier file published (or
//! that consumed an overlay entry from a file that itself failed validation)
//! may have observed a hit/miss pattern serial checking would not produce; its
//! speculative result is discarded and the file is re-checked serially against
//! the now-committed cache state. By induction over file order the committed
//! results and final cache contents are byte-identical to a serial run.
//!
//! Conflict keys are equality-consistent structural digests
//! ([`surge_ts_types::type_conflict_digest`]); a digest collision only causes
//! a spurious (sound) recheck, never a missed conflict.
//!
//! The session is installed per worker *thread* (the `ACTIVE_TYPE_STORE`
//! pattern) rather than plumbed through `CheckerContext`, because resolution
//! can materialize shadow contexts from captured declaration environments
//! mid-check ([`crate::context::DeclarationEnvironmentHandle::checker_context`]);
//! those clones carry the same live cache `Arc`s and must route through the
//! same overlay. Contexts with deliberately isolated cache handles (e.g. the
//! export-value shadow context) are recognized by pointer identity and keep
//! their isolated behavior.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use surge_ts_types::fx::{FxHashMap, FxHasher};
use surge_ts_types::{FunctionType, Type, type_conflict_digest};

use crate::context::{
    DeclarationResolutionKey, GenericInstantiationCacheEntry, InstantiationCacheEntry,
    InterfaceDeclarationTemplate, InterfaceInstantiationKey, InterfaceMemberInstantiationKey,
    InterfaceOverloadInstantiationKey, StableInterfaceDeclarationId,
};

mod commit;
mod file_log;
mod fingerprint;
mod reservation;
mod session;

pub(crate) use commit::*;
pub(crate) use file_log::*;
pub(crate) use fingerprint::*;
pub(crate) use reservation::*;
pub(crate) use session::*;

type GenericMap = FxHashMap<DeclarationResolutionKey, crate::context::DigestBucket<GenericInstantiationCacheEntry>>;
type InstantiationMap = FxHashMap<DeclarationResolutionKey, crate::context::DigestBucket<InstantiationCacheEntry>>;
type PhysicalMap = FxHashMap<InterfaceInstantiationKey, Arc<Type>>;
type TemplateMap = FxHashMap<StableInterfaceDeclarationId, Arc<InterfaceDeclarationTemplate>>;
type MethodMap = FxHashMap<InterfaceMemberInstantiationKey, FunctionType>;
type OverloadMap = FxHashMap<InterfaceOverloadInstantiationKey, FunctionType>;

const TAG_GENERIC: u8 = 0;
const TAG_INSTANTIATION: u8 = 1;
const TAG_PHYSICAL: u8 = 2;
const TAG_TEMPLATE: u8 = 3;
const TAG_METHOD: u8 = 4;
const TAG_OVERLOAD: u8 = 5;

fn digest_flat<K: Hash>(tag: u8, key: &K) -> u64 {
    let mut hasher = FxHasher::default();
    tag.hash(&mut hasher);
    key.hash(&mut hasher);
    hasher.finish()
}

fn digest_bucket(tag: u8, key: &DeclarationResolutionKey, arguments: &[Type]) -> u64 {
    let mut hasher = FxHasher::default();
    tag.hash(&mut hasher);
    key.hash(&mut hasher);
    arguments.len().hash(&mut hasher);
    for argument in arguments {
        type_conflict_digest(argument).hash(&mut hasher);
    }
    hasher.finish()
}
