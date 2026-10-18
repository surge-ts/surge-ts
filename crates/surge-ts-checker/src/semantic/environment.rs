//! The engine lock and the process-global "current program" state it guards.
//!
//! The checker keeps parts of the program it is checking in process globals
//! (module-resolution policy, the published module scopes and ambient globals,
//! the program-lifetime instantiation memo, the check-phase marker) and in
//! thread-local caches. One check at a time, one program alive at a time, is
//! the only model that state supports, so every check and every query runs
//! under [`engine_lock`] and first claims that state with
//! [`claim_program_state`]; a retained program re-installs its own state each
//! time it is entered and takes the memo back out when it leaves.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use surge_ts_types::Type;
use surge_ts_types::fx::FxHashMap;

use crate::context::{DeclarationEnvironmentStore, DeclarationResolutionKey};
use crate::program::PublishedProgramState;

static ENGINE_LOCK: Mutex<()> = Mutex::new(());
static NEXT_PROGRAM_ID: AtomicU64 = AtomicU64::new(1);
/// The program the process-global state was last claimed for.
static ENGINE_PROGRAM: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// The program whose state this thread's caches were last filled for.
    static THREAD_PROGRAM: Cell<u64> = const { Cell::new(0) };
}

/// Serializes every check and query in the process. A panic inside a query
/// poisons nothing the next one relies on: each entry re-installs its state.
pub(crate) fn engine_lock() -> MutexGuard<'static, ()> {
    ENGINE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn next_program_id() -> u64 {
    NEXT_PROGRAM_ID.fetch_add(1, Ordering::Relaxed)
}

/// Marks the process globals and this thread's caches as belonging to
/// `program`, clearing what another program left in them. A retained program
/// skips the end-of-run teardown, so without this the next program's check
/// would read its file-keyed state: the published per-file module scopes
/// answer a lookup made before the new program publishes its own, with the
/// old program's file-local declaration where the merged global belongs.
pub(crate) fn claim_program_state(program: u64) {
    if ENGINE_PROGRAM.swap(program, Ordering::Relaxed) != program {
        crate::program::set_check_phase(false);
        crate::program::clear_program_module_scopes();
        crate::checks::function::clear_circular_body_returns();
    }
    claim_thread_caches(program);
}

fn claim_thread_caches(program: u64) {
    if THREAD_PROGRAM.with(Cell::get) == program {
        return;
    }
    crate::paths::clear_canonicalize_cache();
    crate::modules::clear_relative_module_cache();
    crate::modules::clear_star_export_unresolved_cache();
    crate::modules::clear_namespace_alias_table_cache();
    crate::infer::types::clear_deferred_merges();
    crate::infer::types::cache::clear_variable_instantiation_memo();
    crate::infer::types::clear_type_name_suggestions();
    crate::checks::function::clear_generic_body_reads();
    crate::symbols::clear_declared_variances();
    THREAD_PROGRAM.with(|current| current.set(program));
}

/// What a retained program installs into the process globals on entry.
pub(crate) struct ProgramGlobals {
    pub(crate) node_esm_files: Option<Arc<HashSet<String>>>,
    pub(crate) allow_arbitrary_extensions: bool,
    pub(crate) published: PublishedProgramState,
    pub(crate) environment_store: Weak<DeclarationEnvironmentStore>,
    /// The program-lifetime instantiation memo, parked here while the program
    /// is not entered.
    pub(crate) memo: Mutex<FxHashMap<DeclarationResolutionKey, Type>>,
}

/// Installs `globals` for the duration of `f`, then parks the memo again.
/// The caller holds the engine lock.
pub(crate) fn with_program_globals<R>(program: u64, globals: &ProgramGlobals, f: impl FnOnce() -> R) -> R {
    struct Exit<'a> {
        globals: &'a ProgramGlobals,
    }
    impl Drop for Exit<'_> {
        fn drop(&mut self) {
            crate::program::set_check_phase(false);
            if let Ok(mut parked) = self.globals.memo.lock() {
                crate::infer::types::cache::swap_program_module_instantiation_memo(&mut parked);
            }
        }
    }
    claim_program_state(program);
    crate::modules::set_node_esm_files(globals.node_esm_files.clone());
    crate::modules::set_allow_arbitrary_extensions(globals.allow_arbitrary_extensions);
    crate::program::install_published_program_state(&globals.published);
    if let Some(store) = globals.environment_store.upgrade() {
        store.mark_program_lifetime();
    }
    if let Ok(mut parked) = globals.memo.lock() {
        crate::infer::types::cache::swap_program_module_instantiation_memo(&mut parked);
    }
    let _exit = Exit { globals };
    crate::program::set_check_phase(true);
    f()
}
