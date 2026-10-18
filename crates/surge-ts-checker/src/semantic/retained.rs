use std::sync::{Arc, Mutex};

use surge_ts_syntax::ParsedSource;
use surge_ts_types::fx::FxHashMap;
use surge_ts_types::{ProgramTypeStore, with_program_type_store};

use super::environment::{ProgramGlobals, claim_thread_caches, engine_lock, next_program_id, with_program_globals};
use super::index::FileSemanticIndex;
use super::query::Query;
use crate::context::{CheckerContext, CheckerOptions, CompatibilityStats, FileKind};
use crate::program::{ProgramCheckResult, RetainedRun, SourceFileInput};

/// A checked program kept alive for queries.
///
/// Its diagnostics are exactly the ones the one-shot check of the same inputs
/// reports; what differs is that the program's state outlives the check.
/// Everything it hands out ([`surge_ts_types::Type`] values in particular) is
/// only meaningful together with this program.
pub struct RetainedProgram {
    id: u64,
    store: Arc<ProgramTypeStore>,
    globals: ProgramGlobals,
    file_names: Vec<String>,
    file_kinds: Vec<FileKind>,
    file_index_by_name: FxHashMap<String, usize>,
    state: Mutex<RetainedState>,
}

pub(crate) struct RetainedState {
    pub(crate) run: RetainedRun,
    /// The context queries run in: a clone of the program's, reused across
    /// files the way the serial check reuses one.
    pub(crate) query_ctx: CheckerContext,
    pub(crate) indexes: FxHashMap<usize, Arc<FileSemanticIndex>>,
    /// The global interfaces primitives' apparent types resolve to.
    pub(crate) apparent_types: FxHashMap<&'static str, surge_ts_types::Type>,
}

impl RetainedProgram {
    /// Checks `files` as [`crate::lowlevel::check_program_with_prescanned_sources`]
    /// does, keeping the program. `None` for an empty program.
    pub fn check(
        files: Vec<SourceFileInput>,
        prescanned: Vec<ParsedSource>,
        options: CheckerOptions,
        jobs: usize,
    ) -> (ProgramCheckResult, Option<RetainedProgram>) {
        let _lock = engine_lock();
        super::index::mark_api_program();
        let id = next_program_id();
        claim_thread_caches(id);
        let node_esm_files = options
            .node_module_resolution
            .then(|| Arc::new(options.esm_module_files.clone()));
        let allow_arbitrary_extensions = options.allow_arbitrary_extensions;
        let store = ProgramTypeStore::new();
        let (result, run) =
            crate::program::check_program_retained(files, prescanned, options, jobs, store.clone());
        let Some(run) = run else {
            return (result, None);
        };
        let mut memo = FxHashMap::default();
        crate::infer::types::cache::swap_program_module_instantiation_memo(&mut memo);
        let module_scopes = crate::program::program_module_scopes();
        let ambient_globals = Arc::new(
            run.ctx
                .ambient_global_symbols
                .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
        );
        let file_names: Vec<String> = run.parsed_files.iter().map(|file| file.file_name.clone()).collect();
        let file_kinds = run.parsed_files.iter().map(|file| file.file_kind).collect();
        let file_index_by_name = file_names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.clone(), index))
            .collect();
        let mut query_ctx = run.ctx.clone();
        query_ctx.diagnostics.clear();
        query_ctx.stats = CompatibilityStats::default();
        let program = RetainedProgram {
            id,
            store,
            globals: ProgramGlobals {
                node_esm_files,
                allow_arbitrary_extensions,
                module_scopes,
                ambient_globals,
                memo: Mutex::new(memo),
            },
            file_names,
            file_kinds,
            file_index_by_name,
            state: Mutex::new(RetainedState {
                run,
                query_ctx,
                indexes: FxHashMap::default(),
                apparent_types: FxHashMap::default(),
            }),
        };
        (result, Some(program))
    }

    /// The program's files in the order the checker loaded them: default
    /// libraries first.
    pub fn file_names(&self) -> &[String] {
        &self.file_names
    }

    pub fn file_index(&self, file_name: &str) -> Option<usize> {
        self.file_index_by_name.get(file_name).copied()
    }

    pub fn file_kind(&self, file_index: usize) -> Option<FileKind> {
        self.file_kinds.get(file_index).copied()
    }

    /// Runs `f` with the program installed as the engine's current program.
    /// Queries from every program in the process are serialized.
    pub fn query<R>(&self, f: impl FnOnce(&mut Query<'_>) -> R) -> R {
        let _lock = engine_lock();
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        with_program_globals(self.id, &self.globals, || {
            with_program_type_store(self.store.clone(), || {
                let mut query = Query::new(self, &mut state);
                f(&mut query)
            })
        })
    }
}
