//! Named-type resolution memoization and declaration resolution keys.

use super::*;

use std::path::Path;
use std::sync::Arc;

use surge_ts_types::{Type, TypeReference};

use crate::context::{
    CheckerContext, DeclarationNamespace, DeclarationResolutionKey, DeclarationResolutionState,
    GenericInstantiationCacheEntry, InstantiationCacheEntry,
};
use crate::symbols::TypeDeclarationInfo;

mod lazy_annotation;
mod lazy_instantiation;
mod lazy_member;
mod module_memo;
mod physical_interface;

pub(crate) use lazy_annotation::*;
pub(crate) use lazy_instantiation::*;
pub(crate) use lazy_member::*;
pub(crate) use module_memo::*;
pub(crate) use physical_interface::*;

pub(crate) fn type_declaration_resolution_key(
    declaration: &TypeDeclarationInfo,
) -> DeclarationResolutionKey {
    match declaration {
        TypeDeclarationInfo::Alias(alias) => alias_resolution_key(alias),
        TypeDeclarationInfo::Interface(interface) => interface_resolution_key(interface),
    }
}

pub(crate) fn alias_resolution_key(
    alias: &crate::symbols::TypeAliasInfo,
) -> DeclarationResolutionKey {
    alias
        .cached_resolution_key
        .get_or_init(|| declaration_resolution_key(&alias.file_name, &alias.name))
        .clone()
}

pub(crate) fn interface_resolution_key(
    interface: &crate::symbols::InterfaceInfo,
) -> DeclarationResolutionKey {
    interface
        .cached_resolution_key
        .get_or_init(|| declaration_resolution_key(&interface.file_name, &interface.name))
        .clone()
}

pub(crate) fn declaration_resolution_key(file_name: &str, name: &str) -> DeclarationResolutionKey {
    DeclarationResolutionKey {
        file_name: canonical_declaration_file_name(file_name),
        name: Arc::from(name),
        namespace: DeclarationNamespace::Type,
        fingerprint: 0,
    }
}

pub(crate) fn type_declaration_alias_id(
    declaration: &TypeDeclarationInfo,
    key: &DeclarationResolutionKey,
) -> Arc<str> {
    let build = || Arc::from(format!("{}\u{0}{}", key.file_name, key.name));
    match declaration {
        TypeDeclarationInfo::Alias(alias) => alias.cached_alias_id.get_or_init(build).clone(),
        TypeDeclarationInfo::Interface(interface) => {
            interface.cached_alias_id.get_or_init(build).clone()
        }
    }
}

thread_local! {
    /// Bumped every time the named-type memo reports a declaration that is
    /// *mid-resolution on another frame* as degraded. That answer depends on
    /// which resolutions are in flight, not on the declaration, so any
    /// expansion that observed one is not a pure function of its own inputs
    /// and must not be memoized for reuse at a different site.
    static IN_FLIGHT_DEGRADED_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub(crate) fn in_flight_degraded_read_epoch() -> u64 {
    IN_FLIGHT_DEGRADED_READS.get()
}

pub(crate) fn note_in_flight_degraded_read() {
    IN_FLIGHT_DEGRADED_READS.with(|reads| reads.set(reads.get().wrapping_add(1)));
}

pub(crate) fn get_cached_named_type_resolution(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
    resolving: &[DeclarationResolutionKey],
) -> Option<ResolvedType> {
    let cache = ctx.resolved_named_types.lock().ok()?;

    match cache.get(key) {
        Some(DeclarationResolutionState::Resolved { ty, had_error }) => {
            crate::program::record_program_counter(|c| c.named_type_cache_hit_count += 1);
            Some(ResolvedType {
                ty: ty.clone(),
                had_error: *had_error,
            })
        }
        Some(DeclarationResolutionState::Resolving) => {
            if resolving.iter().any(|current| current == key) {
                None
            } else {
                note_in_flight_degraded_read();
                Some(ResolvedType {
                    ty: Type::Unknown,
                    had_error: true,
                })
            }
        }
        None => None,
    }
}

/// Whether `key` is being resolved further up some resolution in flight.
pub(crate) fn named_type_resolution_in_progress(ctx: &CheckerContext, key: &DeclarationResolutionKey) -> bool {
    ctx.resolved_named_types
        .lock()
        .is_ok_and(|cache| matches!(cache.get(key), Some(DeclarationResolutionState::Resolving)))
}

pub(crate) fn mark_named_type_resolution_in_progress(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
) {
    if let Ok(mut cache) = ctx.resolved_named_types.lock() {
        cache.insert(key.clone(), DeclarationResolutionState::Resolving);
    }
}

/// Drops the in-progress marker of a resolution whose answer is not stored,
/// so the next read resolves the declaration afresh.
pub(crate) fn forget_named_type_resolution(ctx: &CheckerContext, key: &DeclarationResolutionKey) {
    if let Ok(mut cache) = ctx.resolved_named_types.lock() {
        cache.remove(key);
    }
}

pub(crate) fn cache_named_type_resolution(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
    resolved: &ResolvedType,
) {
    if let Ok(mut cache) = ctx.resolved_named_types.lock() {
        crate::program::record_program_counter(|c| c.named_type_cache_insert_count += 1);
        cache.insert(
            key.clone(),
            DeclarationResolutionState::Resolved {
                ty: resolved.ty.clone(),
                had_error: resolved.had_error,
            },
        );
    }
}

/// Upper bound on distinct instantiations memoized per generic declaration — a
/// defensive guard against a pathological declaration accumulating an unbounded
/// bucket that linear-search would have to scan. Sized for user utility aliases
/// (`Omit`, `Identity`, …), which accumulate hundreds of distinct argument
/// tuples on a large project; an entry evicted by a lower cap is re-expanded at
/// every remaining reference, which dominates checking time and peak memory
/// (measured on zod at the previous cap of 64).
const GENERIC_INSTANTIATION_BUCKET_CAP: usize = 4096;

/// Effective per-declaration bucket cap, overridable via
/// `SURGE_GENERIC_CACHE_BUCKET_CAP` for cache-bound experiments and the
/// bounded-vs-unbounded regression tests. Over-cap entries are simply not
/// cached (re-expanded on demand), so any cap produces identical diagnostics —
/// only time/memory change. Read once per process.
pub(crate) fn generic_instantiation_bucket_cap() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SURGE_GENERIC_CACHE_BUCKET_CAP")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(GENERIC_INSTANTIATION_BUCKET_CAP)
    })
}

pub(crate) fn get_persistent_generic_resolution(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
    arguments: &[Type],
) -> Option<ResolvedType> {
    let resolved = if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_generic(&ctx.program_resolved_generic_types))
    {
        session
            .generic_lookup(key, arguments)
            .map(|(ty, had_error)| ResolvedType { ty, had_error })
    } else {
        let digest = crate::context::arguments_digest(arguments);
        ctx.program_resolved_generic_types
            .lock()
            .ok()
            .and_then(|cache| {
                cache.get(key)?.iter().find_map(|entry| {
                    entry.matches(digest, arguments).then(|| ResolvedType {
                        ty: entry.ty.clone(),
                        had_error: entry.had_error,
                    })
                })
            })
    };
    crate::program::record_program_counter(|c| {
        if resolved.is_some() {
            c.generic_type_cache_hit_count += 1;
        } else {
            c.generic_type_cache_miss_count += 1;
        }
    });
    resolved
}

pub(crate) fn cache_persistent_generic_resolution(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
    arguments: Vec<Type>,
    resolved: &ResolvedType,
) {
    // A generic body read is not program state: what it resolves over its own
    // type variables must not seed what later resolutions find.
    if crate::checks::function::in_generic_body_read() {
        return;
    }
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_generic(&ctx.program_resolved_generic_types))
    {
        session.generic_insert(
            key,
            arguments,
            resolved.ty.clone(),
            resolved.had_error,
            generic_instantiation_bucket_cap(),
        );
        return;
    }
    if let Ok(mut cache) = ctx.program_resolved_generic_types.lock() {
        let digest = crate::context::arguments_digest(&arguments);
        let bucket = cache.entry(key.clone()).or_default();
        if bucket.iter().any(|entry| entry.matches(digest, &arguments)) {
            return;
        }
        if bucket.len() >= generic_instantiation_bucket_cap() {
            crate::program::record_program_counter(|c| c.generic_type_cache_capped_count += 1);
            return;
        }
        crate::program::record_program_counter(|c| c.generic_type_cache_insert_count += 1);
        bucket.push(GenericInstantiationCacheEntry {
            arguments,
            arguments_digest: digest,
            ty: resolved.ty.clone(),
            had_error: resolved.had_error,
        });
    }
}

pub(crate) fn canonical_declaration_file_name(file_name: &str) -> Arc<str> {
    crate::paths::canonicalize_if_exists_arc(Path::new(file_name))
}

/// Interns the structural expansion of `key` at `arguments`, returning the
/// shared `Arc<Type>`. On a hit the previously-expanded shape is returned and
/// `structural` is discarded, so each unique instantiation expands at most once.
pub(crate) fn intern_instantiation(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
    arguments: &[Type],
    structural: Type,
) -> Arc<Type> {
    // A recursive alias's back-edge resolves to a lazy reference to this very
    // instantiation. Stored, every later peel would read itself back and answer
    // the sentinel (`LazyInstantiation::is_self_reference`), so the value is
    // returned without being interned.
    if crate::checks::function::in_generic_body_read() {
        return lookup_instantiation(ctx, key, arguments).map_or_else(|| Arc::new(structural), |entry| entry.resolved);
    }
    if is_own_back_edge(key, arguments, &structural) {
        return Arc::new(structural);
    }
    // A body read for its return reads the program's instantiations but adds
    // to its own, which die with the read.
    if let Some(overlay) = read_only_overlay() {
        if let Some(entry) = lookup_program_instantiation(ctx, key, arguments) {
            return entry.resolved;
        }
        let Ok(mut cache) = overlay.lock() else {
            return Arc::new(structural);
        };
        return intern_into(&mut cache, key, arguments, structural);
    }
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_instantiations(&ctx.program_instantiations))
    {
        return session.instantiation_intern(
            key,
            arguments,
            structural,
            generic_instantiation_bucket_cap(),
        );
    }
    let Ok(mut cache) = ctx.program_instantiations.lock() else {
        return Arc::new(structural);
    };
    intern_into(&mut cache, key, arguments, structural)
}

fn is_own_back_edge(key: &DeclarationResolutionKey, arguments: &[Type], structural: &Type) -> bool {
    matches!(structural, Type::Reference(reference)
        if reference.arguments.as_ref() == arguments
            && reference.id.len() == key.file_name.len() + 1 + key.name.len()
            && reference.id.starts_with(key.file_name.as_ref())
            && reference.id.ends_with(key.name.as_ref()))
}

type InstantiationMap = surge_ts_types::fx::FxHashMap<DeclarationResolutionKey, Vec<InstantiationCacheEntry>>;

fn intern_into(
    cache: &mut InstantiationMap,
    key: &DeclarationResolutionKey,
    arguments: &[Type],
    structural: Type,
) -> Arc<Type> {
    let digest = crate::context::arguments_digest(arguments);
    let bucket = cache.entry(key.clone()).or_default();
    if let Some(entry) = bucket.iter().find(|entry| entry.matches(digest, arguments)) {
        crate::program::record_program_counter(|c| c.instantiation_intern_hit_count += 1);
        return entry.resolved.clone();
    }
    let resolved = Arc::new(structural);
    if bucket.len() < generic_instantiation_bucket_cap() {
        crate::program::record_program_counter(|c| c.instantiation_intern_insert_count += 1);
        bucket.push(InstantiationCacheEntry {
            arguments: arguments.to_vec(),
            arguments_digest: digest,
            resolved: resolved.clone(),
        });
    } else {
        crate::program::record_program_counter(|c| c.instantiation_intern_capped_count += 1);
    }
    resolved
}

/// Looks up a previously-interned instantiation without expanding anything.
pub(crate) fn lookup_instantiation(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
    arguments: &[Type],
) -> Option<InstantiationCacheEntry> {
    lookup_program_instantiation(ctx, key, arguments).or_else(|| lookup_read_only_overlay(key, arguments))
}

fn lookup_program_instantiation(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
    arguments: &[Type],
) -> Option<InstantiationCacheEntry> {
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_instantiations(&ctx.program_instantiations))
    {
        return session.instantiation_lookup(key, arguments);
    }
    let digest = crate::context::arguments_digest(arguments);
    let cache = ctx.program_instantiations.lock().ok()?;
    cache
        .get(key)?
        .iter()
        .find(|entry| entry.matches(digest, arguments))
        .cloned()
}

/// Deferral-aware instantiation lookup for the lazy peel: distinguishes a
/// genuine miss from a deferral (the key is owned by an earlier not-yet-committed
/// replay publisher). Only a live deferring replay session ever returns
/// `Deferred`; every other context sees `Hit`/`Miss` exactly as
/// [`lookup_instantiation`].
pub(crate) fn lookup_instantiation_probe(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
    arguments: &[Type],
) -> crate::speculative::InstantiationProbe {
    use crate::speculative::InstantiationProbe;
    if let Some(session) = crate::speculative::active_check_session()
        .filter(|session| session.owns_instantiations(&ctx.program_instantiations))
    {
        return session.instantiation_probe(key, arguments);
    }
    let digest = crate::context::arguments_digest(arguments);
    match ctx.program_instantiations.lock().ok().and_then(|cache| {
        cache
            .get(key)
            .and_then(|bucket| bucket.iter().find(|entry| entry.matches(digest, arguments)))
            .cloned()
    }) {
        Some(entry) => InstantiationProbe::Hit(entry),
        None => lookup_read_only_overlay(key, arguments).map_or(InstantiationProbe::Miss, InstantiationProbe::Hit),
    }
}

thread_local! {
    static READ_ONLY_OVERLAYS: std::cell::RefCell<Vec<Arc<std::sync::Mutex<InstantiationMap>>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn read_only_overlay() -> Option<Arc<std::sync::Mutex<InstantiationMap>>> {
    READ_ONLY_OVERLAYS.with(|overlays| overlays.borrow().last().cloned())
}

fn lookup_read_only_overlay(key: &DeclarationResolutionKey, arguments: &[Type]) -> Option<InstantiationCacheEntry> {
    let overlay = read_only_overlay()?;
    let cache = overlay.lock().ok()?;
    let digest = crate::context::arguments_digest(arguments);
    cache.get(key)?.iter().find(|entry| entry.matches(digest, arguments)).cloned()
}

/// Runs `f` as a read that sees the program's instantiations but interns what
/// it expands into a map of its own, dropped when the read ends; a nested read
/// starts its own. Without one, every repeat of an instantiation inside the
/// read expanded again from nothing.
pub(crate) fn with_read_only_instantiations<R>(f: impl FnOnce() -> R) -> R {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            READ_ONLY_OVERLAYS.with(|overlays| overlays.borrow_mut().pop());
        }
    }
    READ_ONLY_OVERLAYS.with(|overlays| overlays.borrow_mut().push(Arc::default()));
    let _leave = Leave;
    f()
}

/// Builds a lazy/nominal [`Type::Reference`] over a shared structural expansion.
/// `id` is the nominal identity (`file\u{0}name`), `display` the diagnostic form
/// (e.g. `Box<string>`), and `arguments` the resolved type arguments.
#[allow(dead_code)]
pub(crate) fn make_type_reference(
    id: impl Into<Arc<str>>,
    display: impl Into<Arc<str>>,
    arguments: Vec<Type>,
    resolved: Arc<Type>,
) -> Type {
    Type::Reference(TypeReference::new(
        id,
        display,
        arguments,
        Arc::new(InternedInstantiation { resolved }),
    ))
}
