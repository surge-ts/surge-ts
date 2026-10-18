use surge_ts_types::Type;

use crate::context::{
    CheckerContext, DeclarationNamespace, DeclarationResolutionKey, DeclarationResolutionState,
};
use crate::infer::types::*;

/// Module-scoped instantiation memo: the same interface declaration expanded
/// again under the *same* substitution, inside the same module-analysis (or
/// file-check) region that owns `resolved_named_types`.
///
/// The eager expansion of a mutually recursive user interface cluster is
/// combinatorial: zod's v3 `ZodType` hierarchy re-expands `ZodEffectsDef` 47.5k
/// times per run under only three distinct substitutions, because the cluster
/// degrades (an unmodelled `enum` member type in `typeName`) and a degraded
/// expansion is uncacheable program-wide by design. This tier is not
/// program-wide: it lives in the same map, with the same lifetime and the same
/// environment stamp, as the non-generic named-type memo that already stores
/// `had_error` results — `replace_resolved_named_types` drops it whenever the
/// owning module or file changes, so a degraded shape can never outlive the
/// scope that produced it.
///
/// Entries are stored under [`DeclarationNamespace::TypeSignatureContext`] with
/// a substitution fingerprint appended to the name, so they cannot collide with
/// the non-generic entries the same map holds under `Type`.
/// Set on a module-instantiation memo key's fingerprint so it can never collide
/// with the display-tagged signature-context keys, which share the namespace.
pub(super) const MODULE_INSTANTIATION_MEMO_TAG: u64 = 1 << 63;

pub(crate) fn module_instantiation_memo_key(
    declaration_key: &DeclarationResolutionKey,
    fingerprint: u64,
) -> DeclarationResolutionKey {
    DeclarationResolutionKey {
        file_name: declaration_key.file_name.clone(),
        name: declaration_key.name.clone(),
        namespace: DeclarationNamespace::TypeSignatureContext,
        // Distinguished by the fingerprint field rather than by formatting it
        // into the name: this runs once per interface/alias resolution.
        fingerprint: fingerprint | MODULE_INSTANTIATION_MEMO_TAG,
    }
}

/// Program-lifetime companion to the module-scoped instantiation memo, holding
/// **clean expansions only**. Default on since 2026-09-12
/// (`SURGE_IFACE_MEMO_PROGRAM=0` turns it off, `=check` restricts it to the
/// check phase).
///
/// The module scoping of the primary memo is deliberate — see the tier's doc
/// above: dropping the map on every module/file change is what stops a degraded
/// (`had_error`) shape outliving the scope that produced it, which the
/// memory-lifetime rules require. So this companion never stores a degraded
/// result; only the `had_error == false` expansions, whose fingerprint already
/// pins phase, scope openness, stage, attempt, module scope and scope layers.
///
/// Three conditions had to hold before this could share across modules, each
/// found by the memo moving a diagnostic rather than by inspection:
///
/// * the fingerprint must not carry the consumer's per-module declaration-table
///   instance id where the body provably does not read that table
///   (`interface_memo_table_identity_dropped`), or no two modules ever agree on
///   a key;
/// * a body expanded under a shadow context (`values.rs`) must not be stored:
///   the shadow's environment store dies with it, and every later peel of a
///   lazy reference captured there degrades to `Unknown`
///   (`DeclarationEnvironmentStore::is_program_lifetime`);
/// * a body whose HERITAGE re-entered an outer in-progress frame must not be
///   stored — its inherited surface is incomplete in a caller-dependent way —
///   while a member annotation that did so only embeds a nominal cycle
///   reference and stays a function of its key (`resolve_interface`).
///
/// Measured with all three in place (2026-09-12, six corpora byte-identical):
/// tanstack-query 40.0G → 6.0G instructions and 1.03 GB → 189 MB peak
/// footprint, trpc −11% / −30%, zod −12%.
pub(super) static PROGRAM_MODULE_INSTANTIATION_MEMO: std::sync::OnceLock<
    std::sync::Mutex<surge_ts_types::fx::FxHashMap<DeclarationResolutionKey, Type>>,
> = std::sync::OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ProgramMemoMode {
    Off,
    /// `=check`: share only while the check phase runs. Every module has been
    /// bound by then, so two consumers cannot disagree about how much of the
    /// program exists — which is the failure the analysis phase exhibits.
    CheckPhase,
    /// The default: share in every phase.
    All,
}

pub(super) fn program_module_memo_mode() -> ProgramMemoMode {
    static MODE: std::sync::OnceLock<ProgramMemoMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(
        || match std::env::var("SURGE_IFACE_MEMO_PROGRAM").as_deref().ok() {
            Some("0") => ProgramMemoMode::Off,
            Some("check") => ProgramMemoMode::CheckPhase,
            _ => ProgramMemoMode::All,
        },
    )
}

pub(super) fn program_module_memo_enabled() -> bool {
    match program_module_memo_mode() {
        ProgramMemoMode::Off => false,
        ProgramMemoMode::CheckPhase => crate::program::in_check_phase(),
        ProgramMemoMode::All => true,
    }
}

pub(super) fn program_module_memo()
-> &'static std::sync::Mutex<surge_ts_types::fx::FxHashMap<DeclarationResolutionKey, Type>> {
    PROGRAM_MODULE_INSTANTIATION_MEMO
        .get_or_init(|| std::sync::Mutex::new(surge_ts_types::fx::FxHashMap::default()))
}

/// Exchanges the program-lifetime memo with `parked`: a retained program
/// takes its memo out of the process global when a query leaves it, so the
/// next program starts from its own.
pub(crate) fn swap_program_module_instantiation_memo(
    parked: &mut surge_ts_types::fx::FxHashMap<DeclarationResolutionKey, Type>,
) {
    if let Ok(mut memo) = program_module_memo().lock() {
        std::mem::swap(&mut *memo, parked);
    }
}

pub(crate) fn clear_program_module_instantiation_memo() {
    if let Some(memo) = PROGRAM_MODULE_INSTANTIATION_MEMO.get()
        && let Ok(mut memo) = memo.lock()
    {
        memo.clear();
    }
}

pub(crate) fn get_module_instantiation_memo(
    ctx: &CheckerContext,
    key: &DeclarationResolutionKey,
) -> Option<ResolvedType> {
    if let Ok(cache) = ctx.resolved_named_types.lock()
        && let Some(DeclarationResolutionState::Resolved { ty, had_error }) = cache.get(key)
    {
        return Some(ResolvedType {
            ty: ty.clone(),
            had_error: *had_error,
        });
    }
    if program_module_memo_enabled() && !program_memo_excluded(key) {
        if let Ok(memo) = program_module_memo().lock()
            && let Some(ty) = memo.get(key)
        {
            crate::program::record_program_counter(|c| c.program_memo_hit_count += 1);
            return Some(ResolvedType {
                ty: ty.clone(),
                had_error: false,
            });
        }
        crate::program::record_program_counter(|c| c.program_memo_miss_count += 1);
        if program_memo_dump_enabled() {
            eprintln!(
                "[program-memo-miss] {} in {} fp={:x} reader={} check={}",
                key.name,
                key.file_name.rsplit('/').next().unwrap_or(""),
                key.fingerprint,
                ctx.file_name.rsplit('/').next().unwrap_or(""),
                crate::program::in_check_phase()
            );
        }
    }
    None
}

thread_local! {
    /// Expansions over a generic body's own type variables, keyed like the
    /// module memo with the variables' owners (and what they are bound to)
    /// folded into the fingerprint. They stay out of `resolved_named_types`,
    /// which declaration environments capture and so outlives the body: an
    /// entry here is dropped once one of its owners' scopes has ended, since
    /// no later key can name that owner as active again.
    static VARIABLE_INSTANTIATION_MEMO: std::cell::RefCell<
        surge_ts_types::fx::FxHashMap<DeclarationResolutionKey, (std::sync::Arc<[u32]>, ResolvedType)>,
    > = std::cell::RefCell::new(Default::default());
}

const VARIABLE_INSTANTIATION_MEMO_CAP: usize = 8192;
const VARIABLE_INSTANTIATION_MEMO_SWEEP: usize = 256;

pub(crate) fn get_variable_instantiation_memo(key: &DeclarationResolutionKey) -> Option<ResolvedType> {
    VARIABLE_INSTANTIATION_MEMO.with(|memo| memo.borrow().get(key).map(|(_, resolved)| resolved.clone()))
}

pub(crate) fn store_variable_instantiation_memo(
    key: DeclarationResolutionKey,
    owners: std::sync::Arc<[u32]>,
    resolved: &ResolvedType,
) {
    VARIABLE_INSTANTIATION_MEMO.with(|memo| {
        let mut memo = memo.borrow_mut();
        // Sweep entries whose body has ended every so often, not only at the
        // cap: each holds a whole expansion alive.
        if memo.len() % VARIABLE_INSTANTIATION_MEMO_SWEEP == VARIABLE_INSTANTIATION_MEMO_SWEEP - 1 {
            memo.retain(|_, (owners, _)| {
                owners.iter().all(|owner| surge_ts_types::type_variable::is_owner_active(*owner))
            });
        }
        if memo.len() >= VARIABLE_INSTANTIATION_MEMO_CAP {
            memo.clear();
        }
        memo.insert(key, (owners, resolved.clone()));
    });
}

pub(crate) fn clear_variable_instantiation_memo() {
    VARIABLE_INSTANTIATION_MEMO.with(|memo| memo.borrow_mut().clear());
}

/// Opt-in (`SURGE_PROGRAM_MEMO_DUMP=1`): one line per program-memo miss and
/// store, for finding which fingerprint component keeps a type graph's
/// consumers from sharing an expansion.
pub(crate) fn program_memo_dump_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("SURGE_PROGRAM_MEMO_DUMP").is_some())
}

/// Upper bound on entries one module-scoped memo map may hold, overridable via
/// `SURGE_MODULE_MEMO_CAP`. An over-cap expansion is simply not stored (it is
/// re-derived on demand), so any cap produces identical diagnostics — only time
/// and memory change. Sized well above what a large module needs: zod's whole
/// run distinguishes 11.4k `(declaration, substitution)` pairs across *every*
/// module, so the cap exists to bound a pathological declaration rather than to
/// shape normal projects.
pub(super) const MODULE_INSTANTIATION_MEMO_CAP: usize = 65_536;

pub(super) fn module_instantiation_memo_cap() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SURGE_MODULE_MEMO_CAP")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(MODULE_INSTANTIATION_MEMO_CAP)
    })
}

/// EXPERIMENT KNOB (`SURGE_IFACE_MEMO_EXCLUDE=<substr>[,<substr>...]`): keep
/// declarations whose key's file name contains any listed substring out of the
/// program-lifetime memo. Used to bisect which declarations a cross-consumer
/// share is answering differently.
pub(super) fn program_memo_excluded(key: &DeclarationResolutionKey) -> bool {
    static EXCLUDES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let excludes = EXCLUDES.get_or_init(|| {
        std::env::var("SURGE_IFACE_MEMO_EXCLUDE")
            .map(|value| {
                value
                    .split(',')
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    });
    if excludes.is_empty() {
        return false;
    }
    excludes
        .iter()
        .any(|needle| key.file_name.contains(needle.as_str()) || key.name.contains(needle.as_str()))
}

pub(crate) fn store_module_instantiation_memo(
    ctx: &CheckerContext,
    key: DeclarationResolutionKey,
    resolved: &ResolvedType,
    base_is_open: bool,
) {
    // A shadow context's environment store dies with the shadow, so a body
    // expanded there carries lazy references that peel to `Unknown` for every
    // later reader (observed as `Variable` losing its inherited `scope`).
    //
    // `base_is_open` (a heritage base that resolved to `Any`/`Unknown`/errored)
    // makes the expansion consumer-dependent: whether the base resolved at all
    // depends on what the triggering module could see, so sharing it across
    // consumers swaps which of `@typescript-eslint`'s `Variable` /
    // `TSESTree.Identifier` loses its inherited members.
    if program_module_memo_enabled()
        && ctx.declaration_environment_store.is_program_lifetime()
        && !resolved.had_error
        && !base_is_open
        && !program_memo_excluded(&key)
        && let Ok(mut memo) = program_module_memo().lock()
    {
        crate::program::record_program_counter(|c| c.program_memo_store_count += 1);
        if program_memo_dump_enabled() {
            eprintln!(
                "[program-memo-store] {} in {} fp={:x} writer={} check={}",
                key.name,
                key.file_name.rsplit('/').next().unwrap_or(""),
                key.fingerprint,
                ctx.file_name.rsplit('/').next().unwrap_or(""),
                crate::program::in_check_phase()
            );
        }
        memo.insert(key.clone(), resolved.ty.clone());
    }
    if let Ok(mut cache) = ctx.resolved_named_types.lock() {
        if cache.len() >= module_instantiation_memo_cap() {
            return;
        }
        cache.insert(
            key,
            DeclarationResolutionState::Resolved {
                ty: resolved.ty.clone(),
                had_error: resolved.had_error,
            },
        );
    }
}
