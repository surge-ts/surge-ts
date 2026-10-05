use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use surge_ts_diagnostics::Diagnostic;
use surge_ts_types::with_program_type_store;

// Instrumentation lives in `metrics`; re-export it so existing callers that
// reference `crate::program::record_*` / `ProgramTimings` keep resolving and so
// the bare `record_*` calls throughout this file stay in scope.
use crate::metrics::*;

use super::{
    FileCheckResult, ParsedProgramFile, ProgramCheckSharedState, census_check_milestone,
    check_program_file_statements, clone_type_declaration_table,
    collect_function_signatures_from_statements, count_local_type_declarations_in_statements,
    apply_comment_directives, emit_unsupported_declaration_diagnostics,
    extend_diagnostics_dedup, module_scope_declared_names,
};
use crate::context::{CheckerContext, CompatibilityStats, FileKind};
use crate::driver::validate_direct_utility_aliases;
use crate::driver::validate_local_type_declarations;
use crate::symbols::{TypeDeclarationScope, clone_symbol_info_handle};

pub(super) fn check_program_files_serial(
    parsed_files: &mut [ParsedProgramFile],
    shared_state: &mut ProgramCheckSharedState,
    ctx: &CheckerContext,
    timings: Option<Arc<Mutex<ProgramTimings>>>,
    retain: bool,
) -> Vec<FileCheckResult> {
    let mut results = Vec::with_capacity(parsed_files.len());

    // One reused context for the whole pass, exactly like a single parallel
    // worker: `check_program_file` already isolates per-file state (see
    // `begin_file_check`), and serial/parallel diagnostic equality is an
    // asserted invariant. Cloning the (large) context per file was a measured
    // ~3% of check-phase time on tRPC.
    let mut local_ctx = ctx.clone();
    local_ctx.diagnostics.clear();
    local_ctx.stats = CompatibilityStats::default();

    // SURGE_CHECK_CACHE_ISOLATION=1: restore the program-wide resolution caches
    // to their pre-file state after every file, so each file observes exactly
    // the analysis-end cache contents. Measures whether any diagnostic depends
    // on cache entries seeded by earlier files' checking (the blocker for
    // deterministic parallel checking). Experiment probe — not a production
    // mode.
    let cache_isolation = std::env::var_os("SURGE_CHECK_CACHE_ISOLATION").is_some();

    let file_count = parsed_files.len();
    for file_index in 0..file_count {
        let cache_snapshot = (cache_isolation && !parsed_files[file_index].statements.is_empty())
            .then(|| {
                (
                    local_ctx
                        .program_resolved_generic_types
                        .lock()
                        .ok()
                        .map(|m| m.clone()),
                    local_ctx
                        .program_instantiations
                        .lock()
                        .ok()
                        .map(|m| m.clone()),
                    local_ctx
                        .physical_interface_instantiations
                        .lock()
                        .ok()
                        .map(|m| m.clone()),
                    local_ctx
                        .physical_interface_declaration_templates
                        .lock()
                        .ok()
                        .map(|m| m.clone()),
                    local_ctx
                        .physical_interface_method_instantiations
                        .lock()
                        .ok()
                        .map(|m| m.clone()),
                    local_ctx
                        .physical_interface_overload_instantiations
                        .lock()
                        .ok()
                        .map(|m| m.clone()),
                )
            });
        let result = check_program_file(
            file_index,
            &parsed_files[file_index],
            shared_state,
            &mut local_ctx,
            timings.as_ref(),
        );
        if let Some(snapshot) = cache_snapshot {
            if let (Some(saved), Ok(mut live)) =
                (snapshot.0, local_ctx.program_resolved_generic_types.lock())
            {
                *live = saved;
            }
            if let (Some(saved), Ok(mut live)) =
                (snapshot.1, local_ctx.program_instantiations.lock())
            {
                *live = saved;
            }
            if let (Some(saved), Ok(mut live)) = (
                snapshot.2,
                local_ctx.physical_interface_instantiations.lock(),
            ) {
                *live = saved;
            }
            if let (Some(saved), Ok(mut live)) = (
                snapshot.3,
                local_ctx.physical_interface_declaration_templates.lock(),
            ) {
                *live = saved;
            }
            if let (Some(saved), Ok(mut live)) = (
                snapshot.4,
                local_ctx.physical_interface_method_instantiations.lock(),
            ) {
                *live = saved;
            }
            if let (Some(saved), Ok(mut live)) = (
                snapshot.5,
                local_ctx.physical_interface_overload_instantiations.lock(),
            ) {
                *live = saved;
            }
        }
        results.push(result);
        if retain {
            continue;
        }
        // The file is fully checked, and per-file program state is only ever
        // read under the file's own index (checking never consults another
        // file's parse tree, analysis, or bindings — cross-file resolution
        // goes through the scope/value maps on the context). Everything
        // index-scoped can therefore free before the next file's checking
        // allocates. Cross-file `Arc`-shared pieces (scope layers, exported
        // symbol handles held by importers' bindings) survive through their
        // remaining owners; only the genuinely-dead remainder frees.
        parsed_files[file_index].statements = Vec::new();
        shared_state.module_analyses[file_index] = None;
        shared_state.module_import_bindings[file_index] = None;
        // Per-file inference churn leaves freed-but-dirty pages that otherwise
        // accumulate against the footprint across the whole phase.
        if (file_index + 1) % 256 == 0 {
            crate::metrics::release_free_memory();
        }
        if let Some(label) = census_check_milestone(file_index + 1, file_count) {
            emit_type_graph_census(
                label,
                Some(&local_ctx),
                &local_ctx.program_type_store,
                CensusExternalRetention::default(),
            );
        }
    }

    results
}

/// Sentinel passed as `jobs` to request automatic worker-count selection.
pub(super) const AUTO_JOBS: usize = 0;

/// Minimum parsed top-level statements assigned to a worker before `AUTO_JOBS`
/// spins up another thread. Per-worker `CheckerContext` clones and thread spawn
/// are not free, so tiny programs stay serial. Statement count is measured after
/// the `skipLibCheck` trim above, so cleared declaration files correctly count as
/// near-zero work. This is a calibration threshold — tune with `bench:test`.
pub(super) const MIN_STATEMENTS_PER_WORKER: usize = 500;

pub(super) fn resolve_worker_count(jobs: usize, parsed_files: &[ParsedProgramFile]) -> usize {
    let file_count = parsed_files.len();
    if file_count <= 1 {
        return 1;
    }

    // Checking is coupled to the shared resolution caches in an order-visible
    // way (whether a file hits an entry seeded by an earlier-checked file can
    // flip a rendered display form), so naive concurrency changes bytes. The
    // parallel path is serial-equivalent regardless: workers speculate against
    // a frozen cache snapshot and a single-threaded commit publishes their
    // insertions in serial file order, re-checking conflicted files (see
    // `crate::speculative`), so an explicit `--jobs N` is byte-identical to
    // `--jobs 1` at any N. AUTO stays serial for now on measured grounds, not
    // correctness ones: the ~4% of files that conflict re-check serially in
    // the ordered commit, holding wall time at parity with serial while
    // spending more CPU and ~+0.27GB peak footprint (tRPC, 10 workers).
    // Flipping AUTO to the parallel path needs the recheck tail pipelined and
    // per-file release wired into the parallel loop first;
    // `SURGE_PARALLEL_CHECK_AUTO=1` opts in for measurement meanwhile.
    let requested = if jobs == AUTO_JOBS {
        if std::env::var_os("SURGE_PARALLEL_CHECK_AUTO").is_some() {
            let total_statements: usize =
                parsed_files.iter().map(|file| file.statements.len()).sum();
            let by_work = total_statements / MIN_STATEMENTS_PER_WORKER;
            let cores = thread::available_parallelism()
                .map(|cores| cores.get())
                .unwrap_or(1);
            cores.min(by_work)
        } else {
            1
        }
    } else {
        jobs
    };

    requested.max(1).min(file_count)
}

pub(super) fn check_program_files_parallel(
    parsed_files: &[ParsedProgramFile],
    shared_state: &ProgramCheckSharedState,
    ctx: &CheckerContext,
    worker_count: usize,
    timings: Option<Arc<Mutex<ProgramTimings>>>,
) -> Vec<FileCheckResult> {
    debug_assert!(worker_count > 1, "serial checking uses the dedicated path");

    // Serial-equivalent speculative checking: workers never write the six
    // order-visible program caches. Each speculates against an immutable
    // snapshot taken here plus a private overlay, recording per file which
    // cache keys it observed missing; the single-threaded commit pass below
    // publishes insertions in serial file order and re-checks any file whose
    // hit/miss pattern a serial run would not have produced, making the final
    // diagnostics and cache contents byte-identical to `--jobs 1`. See
    // `crate::speculative` for the model and its induction argument.
    let live = crate::speculative::LiveCacheHandles::capture(ctx);
    let base = Arc::new(crate::speculative::CacheSnapshots::capture(&live));
    let stc_phase_start = Instant::now();

    let next_index = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);

    let results = thread::scope(|scope| {
        let next_index = &next_index;
        let completed = &completed;
        let mut handles = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let shared_state = shared_state;
            let timings = timings.clone();
            let mut local_ctx = ctx.clone();
            local_ctx.diagnostics.clear();
            local_ctx.stats = CompatibilityStats::default();
            let session = Arc::new(crate::speculative::CheckSession::new(
                live.clone(),
                base.clone(),
            ));

            handles.push(scope.spawn(move || {
                let type_store = local_ctx.program_type_store.clone();
                let worker_results = with_program_type_store(type_store, || {
                    crate::speculative::with_check_session(session.clone(), || {
                        let mut worker_results = Vec::new();
                        loop {
                            // `fetch_add` hands each worker an ascending file
                            // sequence, so a worker's overlay only ever holds
                            // entries from files earlier in serial order than
                            // the one it is checking.
                            let file_index = next_index.fetch_add(1, Ordering::Relaxed);
                            if file_index >= parsed_files.len() {
                                break;
                            }
                            session.begin_file(file_index);
                            worker_results.push(check_program_file(
                                file_index,
                                &parsed_files[file_index],
                                shared_state,
                                &mut local_ctx,
                                timings.as_ref(),
                            ));
                            let completed_count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(label) =
                                census_check_milestone(completed_count, parsed_files.len())
                            {
                                emit_type_graph_census(
                                    label,
                                    Some(&local_ctx),
                                    &local_ctx.program_type_store,
                                    CensusExternalRetention::default(),
                                );
                            }
                        }
                        worker_results
                    })
                });
                (worker_results, session.take_file_logs())
            }));
        }

        let mut outputs = Vec::with_capacity(handles.len());
        for handle in handles {
            outputs.push(
                handle
                    .join()
                    .expect("parallel project checking worker panicked"),
            );
        }
        outputs
    });

    let worker_phase = stc_phase_start.elapsed();
    let commit_phase_start = Instant::now();
    // The fan-out snapshot is only read through worker sessions, all gone once
    // the scope joins; release the six cloned maps before the commit walk.
    drop(base);
    let n = parsed_files.len();
    let mut slots: Vec<Option<FileCheckResult>> = (0..n).map(|_| None).collect();
    let mut worker_logs: Vec<Option<crate::speculative::FileCacheLog>> =
        (0..n).map(|_| None).collect();
    for (worker_results, logs) in results {
        for result in worker_results {
            let file_index = result.file_index;
            slots[file_index] = Some(result);
        }
        for log in logs {
            let file_index = log.file_index;
            worker_logs[file_index] = Some(log);
        }
    }
    let total_misses: usize = worker_logs
        .iter()
        .flatten()
        .map(crate::speculative::FileCacheLog::miss_count)
        .sum();
    crate::speculative::report_conflict_dag(&worker_logs);

    // `SURGE_DEFER=1` (opt-in): honor deferred resolution. Seed a reservation
    // table from the worker logs (each position reserves the keys it publishes),
    // submit conflicts eagerly, and let a replay that reads a key an earlier
    // not-yet-committed publisher owns *defer* — the pipeline requeues it once
    // that publisher commits (bounded by the inline recheck at the frontier),
    // so it re-runs against a committed view instead of over-recursing into a
    // committed conflict. Byte-identical: a deferred attempt is discarded like a
    // stale replay. `SURGE_REPLAY_OFF` (serial recheck) takes precedence for A/B.
    let defer_enabled =
        std::env::var_os("SURGE_DEFER").is_some() && std::env::var_os("SURGE_REPLAY_OFF").is_none();
    let (reservations, defer_stats) = if defer_enabled {
        let mut table = crate::speculative::ReservationTable::new();
        for (position, log) in worker_logs.iter().enumerate() {
            if let Some(log) = log {
                log.reserve_into(&mut table, position, 0);
            }
        }
        (
            Some(Arc::new(std::sync::RwLock::new(table))),
            Some(Arc::new(crate::speculative::DeferralStats::default())),
        )
    } else {
        (None, None)
    };

    // `SURGE_DEFER_DIFF=1` (opt-in, measurement only): classify why replays go
    // stale. For each stale replay's offending miss digest (a miss an earlier
    // position later published), attribute the actual publisher and its commit
    // kind, and test the digest against the worker-log reservation universe —
    // separating worker-log prediction gaps (fixable by reserving on real
    // insert sets) from reservation-mechanism failures. Also diffs each
    // replay/inline-committed position's real insert set against its worker-log
    // prediction. Counters only; resolution is untouched.
    struct DeferDiff {
        reserved_by: surge_ts_types::fx::FxHashMap<u64, usize>,
        predicted: Vec<surge_ts_types::fx::FxHashSet<u64>>,
        published_by: surge_ts_types::fx::FxHashMap<u64, usize>,
        commit_kind: Vec<u8>,
        stale_replays: u64,
        offenders: u64,
        unreserved: u64,
        reserved_by_publisher: u64,
        reserved_by_other: u64,
        reserved_later_or_self: u64,
        publisher_kind: [u64; 4],
        diffed_positions: u64,
        unpredicted_inserts: u64,
        unrealized_predictions: u64,
    }
    impl DeferDiff {
        const KIND_CLEAN: u8 = 1;
        const KIND_REPLAY: u8 = 2;
        const KIND_INLINE: u8 = 3;

        fn record_commit(
            &mut self,
            position: usize,
            kind: u8,
            digests: impl Iterator<Item = u64>,
            published: &surge_ts_types::fx::FxHashSet<u64>,
        ) {
            self.commit_kind[position] = kind;
            let mut actual = surge_ts_types::fx::FxHashSet::default();
            for digest in digests {
                if published.contains(&digest) {
                    self.published_by.entry(digest).or_insert(position);
                }
                actual.insert(digest);
            }
            if kind != Self::KIND_CLEAN {
                self.diffed_positions += 1;
                let predicted = &self.predicted[position];
                self.unpredicted_inserts +=
                    actual.iter().filter(|d| !predicted.contains(d)).count() as u64;
                self.unrealized_predictions +=
                    predicted.iter().filter(|d| !actual.contains(d)).count() as u64;
            }
        }

        fn record_stale(
            &mut self,
            position: usize,
            log: &crate::speculative::FileCacheLog,
            published: &surge_ts_types::fx::FxHashSet<u64>,
        ) {
            self.stale_replays += 1;
            for digest in log.miss_digests() {
                if !published.contains(&digest) {
                    continue;
                }
                self.offenders += 1;
                let publisher = self.published_by.get(&digest).copied();
                if let Some(publisher) = publisher {
                    let kind = self.commit_kind.get(publisher).copied().unwrap_or(0);
                    self.publisher_kind[usize::from(kind.min(3))] += 1;
                }
                match self.reserved_by.get(&digest) {
                    None => self.unreserved += 1,
                    Some(&reserver) if reserver >= position => self.reserved_later_or_self += 1,
                    Some(&reserver) if Some(reserver) == publisher => {
                        self.reserved_by_publisher += 1;
                    }
                    Some(_) => self.reserved_by_other += 1,
                }
            }
        }
    }
    let mut defer_diff = if std::env::var_os("SURGE_DEFER_DIFF").is_some() {
        let mut reserved_by = surge_ts_types::fx::FxHashMap::default();
        let mut predicted = vec![surge_ts_types::fx::FxHashSet::default(); n];
        for (position, log) in worker_logs.iter().enumerate() {
            if let Some(log) = log {
                for digest in log.insert_digests() {
                    reserved_by.entry(digest).or_insert(position);
                    predicted[position].insert(digest);
                }
            }
        }
        Some(DeferDiff {
            reserved_by,
            predicted,
            published_by: surge_ts_types::fx::FxHashMap::default(),
            commit_kind: vec![0u8; n],
            stale_replays: 0,
            offenders: 0,
            unreserved: 0,
            reserved_by_publisher: 0,
            reserved_by_other: 0,
            reserved_later_or_self: 0,
            publisher_kind: [0; 4],
            diffed_positions: 0,
            unpredicted_inserts: 0,
            unrealized_predictions: 0,
        })
    } else {
        None
    };

    // `SURGE_REPLAY_OFF=1` disables pre-replay (every conflict resolves inline —
    // the serial recheck), for interleaved A/B measurement of the pipeline.
    let submit_at = if std::env::var_os("SURGE_REPLAY_OFF").is_some() {
        vec![usize::MAX; n]
    } else if defer_enabled {
        // Eager: submit every predicted conflict at frontier 0. Deferral +
        // requeue handles the imprecision (a conflict-dependent replay defers on
        // its pending dep and is re-run when it commits), so the clean-dependent
        // restriction of `compute_submit_schedule` is not needed.
        let predicted = crate::speculative::predict_conflicts(&worker_logs);
        (0..n)
            .map(|p| if predicted[p] { 0 } else { usize::MAX })
            .collect()
    } else {
        crate::speculative::compute_submit_schedule(&worker_logs)
    };

    // Pipelined ordered-delta replay: publish in strict serial order (the
    // frontier) while a background pool recomputes predicted-conflict files
    // against the live committed store, each launched exactly when its last
    // dependency commits (dependency-driven schedule). A replay that is still
    // stale, or absent, falls back to the inline recheck — the exact old serial
    // behavior, guaranteed valid. See `crate::replay` for the soundness
    // argument (in-order publication makes hit-validation structural).
    let cap = crate::infer::types::cache::generic_instantiation_bucket_cap();
    let mut published = surge_ts_types::fx::FxHashSet::default();
    let mut dirty = surge_ts_types::fx::FxHashSet::default();
    let mut stats = crate::speculative::StcCommitStats::default();
    let mut recheck_ctx: Option<CheckerContext> = None;
    let mut replay_committed = 0u64;
    let mut replay_stale = 0u64;
    let mut inline_only = 0u64;

    struct CheckReplayShared<'a> {
        parsed_files: &'a [ParsedProgramFile],
        shared_state: &'a ProgramCheckSharedState,
        live: crate::speculative::LiveCacheHandles,
        ctx: &'a CheckerContext,
        timings: Option<Arc<Mutex<ProgramTimings>>>,
        reservations: Option<Arc<std::sync::RwLock<crate::speculative::ReservationTable>>>,
        defer_stats: Option<Arc<crate::speculative::DeferralStats>>,
    }
    struct CheckThreadState {
        ctx: CheckerContext,
        store: Arc<surge_ts_types::ProgramTypeStore>,
    }
    struct CheckReplay {
        result: FileCheckResult,
        log: crate::speculative::FileCacheLog,
    }

    let shared = CheckReplayShared {
        parsed_files,
        shared_state,
        live: live.clone(),
        ctx,
        timings: timings.clone(),
        reservations: reservations.clone(),
        defer_stats: defer_stats.clone(),
    };
    let mut finalized_upto = 0usize;
    // `SURGE_CRITPATH=1`: measure the out-of-order-commit ceiling. Capture the
    // conflict DAG before the walk consumes the logs, and time each inline
    // recompute; the critical path is reported after the walk. Run with
    // `SURGE_REPLAY_OFF=1` for clean, uncontended per-conflict weights.
    let critpath_deps = if std::env::var_os("SURGE_CRITPATH").is_some() {
        Some(crate::speculative::compute_conflict_deps(&worker_logs))
    } else {
        None
    };
    let mut conflict_micros: surge_ts_types::fx::FxHashMap<usize, u128> =
        surge_ts_types::fx::FxHashMap::default();

    let replay_stats = crate::replay::run_frontier_pipeline(
        crate::replay::PipelineConfig { n, worker_count },
        &submit_at,
        &shared,
        || {
            let mut ctx = shared.ctx.clone();
            ctx.diagnostics.clear();
            ctx.stats = CompatibilityStats::default();
            let store = ctx.program_type_store.clone();
            CheckThreadState { ctx, store }
        },
        |shared, thread_state, file_index| {
            // A fresh live-reading session and a unique attempt stamp so the
            // replay recomputes correct values instead of dedup-hitting the
            // discarded worker attempt's environments in the weak store.
            let session = match (&shared.reservations, &shared.defer_stats) {
                (Some(table), Some(stats)) => Arc::new(
                    crate::speculative::CheckSession::new_live_reading_deferring(
                        shared.live.clone(),
                        table.clone(),
                        file_index,
                        stats.clone(),
                    ),
                ),
                _ => Arc::new(crate::speculative::CheckSession::new_live_reading(
                    shared.live.clone(),
                )),
            };
            thread_state.ctx.environment_attempt = crate::replay::next_replay_attempt();
            let result = with_program_type_store(thread_state.store.clone(), || {
                crate::speculative::with_check_session(session.clone(), || {
                    session.begin_file(file_index);
                    check_program_file(
                        file_index,
                        &shared.parsed_files[file_index],
                        shared.shared_state,
                        &mut thread_state.ctx,
                        shared.timings.as_ref(),
                    )
                })
            });
            let deferred_until = session.deferred_until();
            let log = session.take_file_logs().pop().unwrap_or_default();
            (CheckReplay { result, log }, deferred_until)
        },
        |_file_index| true,
        |file_index, replay: Option<CheckReplay>, is_final| {
            // Measurement: everything strictly below the frontier is committed,
            // so finalize those reservations to Ready before this position runs.
            if let Some(table) = &reservations
                && let Ok(mut table) = table.write()
            {
                while finalized_upto < file_index {
                    table.finalize(finalized_upto, 1);
                    finalized_upto += 1;
                }
            }
            if let Some(log) = worker_logs[file_index].take() {
                if matches!(
                    crate::speculative::commit_file_log(
                        &live,
                        &log,
                        &mut published,
                        &dirty,
                        cap,
                        &mut stats,
                    ),
                    crate::speculative::CommitVerdict::Clean
                ) {
                    if let Some(diff) = &mut defer_diff {
                        diff.record_commit(
                            file_index,
                            DeferDiff::KIND_CLEAN,
                            log.insert_digests(),
                            &published,
                        );
                    }
                    // The worker's speculative result stands (slots already holds it).
                    return crate::replay::CommitOutcome::Committed;
                }
                // The worker log conflicted and is now consumed; a retry uses the
                // freshly re-submitted replay rather than re-validating it.
            }
            dirty.insert(file_index);
            let had_replay = replay.is_some();
            let mut stale_log = None;
            if let Some(replay) = replay {
                if crate::speculative::commit_replay_log(
                    &live,
                    &replay.log,
                    &mut published,
                    cap,
                    &mut stats,
                ) {
                    if let Some(diff) = &mut defer_diff {
                        diff.record_commit(
                            file_index,
                            DeferDiff::KIND_REPLAY,
                            replay.log.insert_digests(),
                            &published,
                        );
                    }
                    worker_logs[file_index] = None;
                    replay_committed += 1;
                    slots[file_index] = Some(replay.result);
                    return crate::replay::CommitOutcome::Committed;
                }
                stale_log = Some(replay.log);
            }
            // Stale or absent replay: recompute inline against the exact
            // committed<file_index. Moving this recompute onto the pool (return
            // NeedsReplay) was measured *worse* — the coordinator blocks on the
            // pool round-trip behind a queue of (mostly wasted) look-ahead work,
            // ~2x slower than inline. Reaching the parallel critical-path ceiling
            // (SURGE_CRITPATH: ~0.46 s vs ~1.7 s serial) needs the stale replays
            // eliminated (complete reservations), not the recompute relocated.
            let _ = is_final;
            if had_replay {
                replay_stale += 1;
                if let (Some(diff), Some(stale)) = (&mut defer_diff, &stale_log) {
                    diff.record_stale(file_index, stale, &published);
                }
            } else {
                inline_only += 1;
            }
            worker_logs[file_index] = None;
            let local_ctx = recheck_ctx.get_or_insert_with(|| {
                let mut local_ctx = ctx.clone();
                local_ctx.diagnostics.clear();
                local_ctx.stats = CompatibilityStats::default();
                local_ctx.environment_attempt = 1;
                local_ctx
            });
            let session = Arc::new(crate::speculative::CheckSession::new_live_reading(
                live.clone(),
            ));
            let recompute_start = critpath_deps.as_ref().map(|_| Instant::now());
            let result = crate::speculative::with_check_session(session.clone(), || {
                session.begin_file(file_index);
                check_program_file(
                    file_index,
                    &parsed_files[file_index],
                    shared_state,
                    local_ctx,
                    timings.as_ref(),
                )
            });
            if let Some(start) = recompute_start {
                conflict_micros.insert(file_index, start.elapsed().as_micros());
            }
            slots[file_index] = Some(result);
            for recheck_log in session.take_file_logs() {
                crate::speculative::apply_file_log(
                    &live,
                    &recheck_log,
                    &mut published,
                    cap,
                    &mut stats,
                );
                if let Some(diff) = &mut defer_diff {
                    diff.record_commit(
                        file_index,
                        DeferDiff::KIND_INLINE,
                        recheck_log.insert_digests(),
                        &published,
                    );
                }
            }
            crate::replay::CommitOutcome::Committed
        },
    );

    if std::env::var_os("SURGE_STC_STATS").is_some() {
        eprintln!(
            "[stc] files={} clean={} miss_conflicts={} dep_conflicts={} published={} \
             skipped_existing={} cap_blocked={} total_misses={} worker_phase={:.2}s \
             commit_phase={:.2}s",
            stats.files,
            stats.clean_commits,
            stats.miss_conflicts,
            stats.dependency_conflicts,
            stats.published_entries,
            stats.merge_skipped_existing,
            stats.merge_cap_blocked,
            total_misses,
            worker_phase.as_secs_f64(),
            commit_phase_start.elapsed().as_secs_f64(),
        );
        eprintln!(
            "[stc-replay] submitted={} replay_committed={replay_committed} stale={replay_stale} \
             inline_only={inline_only} wasted={} peak_in_flight={}",
            replay_stats.submitted, replay_stats.wasted, replay_stats.peak_in_flight,
        );
    }

    if let Some(deps) = &critpath_deps {
        crate::speculative::report_critical_path(deps, &conflict_micros);
    }
    crate::context::report_local_values_consults(ctx.module_local_values_by_file.len());

    if let Some(diff) = &defer_diff {
        eprintln!(
            "[stc-defer-diff] stale={} offenders={} unreserved={} reserved_by_other={} \
             reserved_by_publisher={} reserved_later_or_self={} \
             publisher_kind(unknown/clean/replay/inline)={}/{}/{}/{}",
            diff.stale_replays,
            diff.offenders,
            diff.unreserved,
            diff.reserved_by_other,
            diff.reserved_by_publisher,
            diff.reserved_later_or_self,
            diff.publisher_kind[0],
            diff.publisher_kind[1],
            diff.publisher_kind[2],
            diff.publisher_kind[3],
        );
        eprintln!(
            "[stc-defer-diff] diffed_positions={} unpredicted_inserts={} unrealized_predictions={}",
            diff.diffed_positions, diff.unpredicted_inserts, diff.unrealized_predictions,
        );
    }

    if let (Some(table), Some(stats)) = (&reservations, &defer_stats)
        && let Ok(mut table) = table.write()
    {
        for position in finalized_upto..n {
            table.finalize(position, 1);
        }
        use std::sync::atomic::Ordering::Relaxed;
        eprintln!(
            "[stc-defer] replay_misses={} would_defer={} pending_leak={} peak_pending={}",
            stats.queried.load(Relaxed),
            stats.deferred.load(Relaxed),
            table.pending_count(),
            table.peak_pending(),
        );
    }

    slots.into_iter().flatten().collect()
}

pub(super) fn module_scope_by_file_map(
    parsed_files: &[ParsedProgramFile],
    module_resolution_scopes: &[Option<Arc<crate::symbols::TypeDeclarationScope>>],
    ctx: &CheckerContext,
) -> surge_ts_types::fx::FxHashMap<Arc<str>, Arc<crate::symbols::TypeDeclarationScope>> {
    let mut map: surge_ts_types::fx::FxHashMap<
        Arc<str>,
        Arc<crate::symbols::TypeDeclarationScope>,
    > = parsed_files
        .iter()
        .zip(module_resolution_scopes.iter())
        .filter_map(|(parsed_file, scope)| {
            scope
                .as_ref()
                .map(|scope| (Arc::from(parsed_file.file_name.as_str()), scope.clone()))
        })
        .collect();
    // A file whose declarations live in ambient `declare module` blocks has an
    // empty (or absent) per-file scope of its own; the ambient scope carries
    // the blocks' declarations and their block-internal import bindings. A
    // file with both top-level declarations and blocks keeps its own layers
    // first.
    for (file_name, ambient_scope) in ctx.ambient_file_type_scopes.iter() {
        match map.get_mut(file_name) {
            Some(existing) => {
                if existing.is_empty() {
                    *existing = ambient_scope.clone();
                } else {
                    let mut layers = existing.layers().to_vec();
                    layers.extend(ambient_scope.layers().iter().cloned());
                    *existing = Arc::new(crate::symbols::TypeDeclarationScope::new(layers));
                }
            }
            None => {
                map.insert(file_name.clone(), ambient_scope.clone());
            }
        }
    }
    map
}

fn emit_bind_diagnostics(parsed_file: &ParsedProgramFile, ctx: &mut CheckerContext) {
    for error in &parsed_file.bind_errors {
        ctx.push(super::diagnostics::parser_error_diagnostic(error, &parsed_file.file_name));
    }
}

/// Codes only tsc's binder reports (its parser reports TS1359 too, but only
/// in a file with syntax errors, which is never bound).
const BINDER_ONLY_CODES: &[u32] = &[
    1100, 1101, 1102, 1210, 1212, 1213, 1214, 1215, 1262, 1314, 1315, 1316, 1344, 1359, 2528,
    2668, 5061, 18012,
];

/// Reconciles surge's own reports with the binder's, which were pushed first.
/// In a bound file the binder alone reports its own codes; for a code tsc's
/// checker reports too, a report surge makes at the same place restates the
/// binder's.
fn reconcile_bind_diagnostics(diagnostics: &mut Vec<Diagnostic>, parsed_file: &ParsedProgramFile) {
    if !parsed_file.tsc_bound {
        return;
    }
    // How many reports the binder made of each code at each place: the
    // binder's own come first, and any beyond them restate one.
    let mut bound: HashMap<(u32, usize), usize> = HashMap::new();
    for error in &parsed_file.bind_errors {
        if let (Some(code), Some(span)) = (error.code, error.span) {
            *bound.entry((code, span.start)).or_default() += 1;
        }
    }
    diagnostics.retain(|diagnostic| {
        let surge_ts_diagnostics::DiagnosticCode::TypeScript(code) = diagnostic.code else {
            return true;
        };
        let Some(span) = diagnostic.span else {
            return !BINDER_ONLY_CODES.contains(&code);
        };
        match bound.get_mut(&(code, span.start)) {
            None => !BINDER_ONLY_CODES.contains(&code),
            Some(0) => false,
            Some(remaining) => {
                *remaining -= 1;
                true
            }
        }
    });
}

/// Turns the parser's grammar findings (see
/// [`surge_ts_syntax::ParsedSource::grammar_diagnostics`]) into diagnostics.
/// The two implicit-`any` kinds are reported only under `noImplicitAny`; the
/// rest are grammar errors tsc reports whatever the options say.
pub(crate) fn emit_grammar_diagnostics(
    findings: &[surge_ts_syntax::ParsedGrammarDiagnostic],
    ctx: &mut CheckerContext,
) {
    for finding in findings {
        if matches!(finding.kind, surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(7027)) {
            ctx.unreachable_statement_runs.push(finding.span);
        }
        if matches!(finding.kind, surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(1294)) && !ctx.options.erasable_syntax_only {
            continue;
        }
        let diagnostic = grammar_finding_diagnostic(finding, ctx);
        if matches!(
            finding.kind,
            surge_ts_syntax::ParsedGrammarDiagnosticKind::NamedSignatureParameterWithoutType
                | surge_ts_syntax::ParsedGrammarDiagnosticKind::ComputedTypeMemberName
                | surge_ts_syntax::ParsedGrammarDiagnosticKind::IndexSignatureKeyReference
                | surge_ts_syntax::ParsedGrammarDiagnosticKind::AsyncReturnTypeReference
                | surge_ts_syntax::ParsedGrammarDiagnosticKind::UnreachableAfterExhaustiveSwitch
                | surge_ts_syntax::ParsedGrammarDiagnosticKind::RestParameterTypeReference
        ) {
            ctx.deferred_grammar_findings.push(finding.clone());
            continue;
        }
        let answered: &'static [u32] = match finding.kind {
            surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(2842) => &[7031],
            surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(2372 | 2373) => &[2304, 2552],
            // The name resolves to the constructor's local in the emitted
            // code, so tsc reports nothing about what it names in the source.
            surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(2301) if diagnostic.is_some() => {
                &[2304, 2552, 2663]
            }
            _ => &[],
        };
        if !answered.is_empty() {
            ctx.grammar_answered_spans
                .push((crate::context::convert_span(finding.span), answered));
        }
        if let Some(diagnostic) = diagnostic {
            ctx.push(diagnostic);
        }
    }
}

/// The grammar findings whose answer needs the file's type declarations, which
/// `emit_grammar_diagnostics` runs before. tsc's `reportImplicitAny` for a
/// call signature, method signature or function type parameter: a bare name
/// that is a type keyword or names a type in scope is TS7051 with the
/// suggested `argN: Name`; otherwise the parameter is the implicit `any` it
/// reads as (TS7006, or TS7019 for a rest parameter).
pub(crate) fn emit_deferred_grammar_diagnostics(ctx: &mut CheckerContext) {
    let findings = std::mem::take(&mut ctx.deferred_grammar_findings);
    for finding in findings {
        if finding.kind == surge_ts_syntax::ParsedGrammarDiagnosticKind::ComputedTypeMemberName {
            if let Some(diagnostic) = computed_type_member_name_diagnostic(&finding, ctx) {
                ctx.push(diagnostic.with_span(crate::context::convert_span(finding.span)));
            }
            continue;
        }
        if finding.kind == surge_ts_syntax::ParsedGrammarDiagnosticKind::UnreachableAfterExhaustiveSwitch {
            let switch = finding.name.as_deref().and_then(|payload| {
                let (start, end) = payload.split_once('\0')?;
                Some((start.parse::<usize>().ok()?, end.parse::<usize>().ok()?))
            });
            if ctx.options.report_unreachable_code
                && switch.is_some_and(|switch| ctx.proven_exhaustive_switches.contains(&switch))
            {
                let diagnostic = Diagnostic::ts7027(ctx.file_name.clone());
                ctx.push(diagnostic.with_span(crate::context::convert_span(finding.span)));
            }
            continue;
        }
        if finding.kind == surge_ts_syntax::ParsedGrammarDiagnosticKind::RestParameterTypeReference {
            if finding.name.as_deref().is_some_and(|name| names_non_array_object_type(name, ctx)) {
                let diagnostic = Diagnostic::ts2370(ctx.file_name.clone());
                ctx.push(diagnostic.with_span(crate::context::convert_span(finding.span)));
            }
            continue;
        }
        if finding.kind == surge_ts_syntax::ParsedGrammarDiagnosticKind::AsyncReturnTypeReference {
            // `checkAsyncFunctionReturnType`: the reference must be to the
            // global `Promise`. A class or interface of another name is not;
            // an alias may resolve to it, and an unresolved name is reported
            // as that.
            let Some((name, argument)) = finding.name.as_deref().and_then(|payload| payload.split_once('\0')) else {
                continue;
            };
            if name != "Promise"
                && matches!(
                    ctx.lookup_type_declaration(name),
                    Some(crate::symbols::TypeDeclarationInfo::Interface(_))
                )
            {
                let awaited = if argument.is_empty() { "void" } else { argument };
                let diagnostic = Diagnostic::ts1064(awaited, ctx.file_name.clone());
                ctx.push(diagnostic.with_span(crate::context::convert_span(finding.span)));
            }
            continue;
        }
        if finding.kind == surge_ts_syntax::ParsedGrammarDiagnosticKind::IndexSignatureKeyReference {
            if index_signature_key_reference_is_invalid(&finding, ctx) {
                let diagnostic = Diagnostic::ts1268(ctx.file_name.clone());
                ctx.push(diagnostic.with_span(crate::context::convert_span(finding.span)));
            }
            continue;
        }
        if !ctx.options.no_implicit_any {
            continue;
        }
        let Some([name, suggested_name, suffix]) = finding
            .name
            .as_deref()
            .map(|payload| payload.split('\0').collect::<Vec<_>>())
            .and_then(|parts| <[&str; 3]>::try_from(parts).ok())
        else {
            continue;
        };
        let file_name = ctx.file_name.clone();
        let diagnostic =
            if is_type_keyword_name(name) || ctx.lookup_type_declaration(name).is_some() {
                Diagnostic::ts7051(suggested_name, format!("{name}{suffix}"), file_name)
            } else if suffix.is_empty() {
                Diagnostic::ts7006(name, file_name)
            } else {
                Diagnostic::ts7019(name, file_name)
            };
        ctx.push(diagnostic.with_span(crate::context::convert_span(finding.span)));
    }
}

/// tsc's `checkComputedPropertyName` for a computed member name, which it
/// checks as an expression: a name that resolves only as a type is TS2693
/// (`checkAndReportErrorForUsingTypeAsValue`), or TS2690 (`K in Keys`) when
/// `maybeMappedType` holds — the member is a type literal's only property and
/// the type is a union of string- and number-like types; a name that resolves
/// to nothing is reported as any unresolved value is. `None` wherever surge
/// cannot tell what the name's declared type is.
fn computed_type_member_name_diagnostic(
    finding: &surge_ts_syntax::ParsedGrammarDiagnostic,
    ctx: &CheckerContext,
) -> Option<Diagnostic> {
    let (name, mapped) = finding.name.as_deref()?.split_once('\0')?;
    if ctx.symbols.get_handle(name).is_some()
        || ctx.ambient_global_symbols.get_handle(name).is_some()
        || ctx.namespace_meaning(name).is_some()
        || crate::checks::expr::is_es2015_or_later_constructor_name(name)
    {
        return None;
    }
    if crate::checks::expr::is_primitive_type_name(name) {
        return Some(Diagnostic::ts2693(name, ctx.file_name.clone()));
    }
    let Some(declaration) = ctx.lookup_type_declaration(name) else {
        return Some(crate::checks::expr::failed_value_name_diagnostic(
            name,
            crate::checks::expr::UnresolvedNameSite::Reference,
            None,
            &ctx.symbols,
            ctx,
        ));
    };
    let is_literal_union = match declaration {
        crate::symbols::TypeDeclarationInfo::Interface(info) => {
            if info.is_class_instance {
                return None;
            }
            false
        }
        crate::symbols::TypeDeclarationInfo::Alias(info) => {
            if info.enum_name.is_some() || !info.body.type_parameters.is_empty() {
                return None;
            }
            declared_literal_union(&info.body.ty)?
        }
    };
    let file_name = ctx.file_name.clone();
    Some(if mapped == "1" && is_literal_union {
        Diagnostic::ts2690(name, if name == "K" { "P" } else { "K" }, file_name)
    } else {
        Diagnostic::ts2693(name, file_name)
    })
}

/// tsc's `checkGrammarIndexSignatureParameters` for a key written as a bare
/// type name: `getTypeFromTypeNode` resolves it, the reference reporting what
/// it gets wrong, and `isValidIndexKeyType` rejects a class or interface, an
/// object, array, function or `boolean` type, and a reference that failed
/// (tsc's `errorType`). Whatever else the name resolves to is left alone.
/// `checkParameter`'s rest rule for a reference to a non-generic class or
/// interface: not assignable to `readonly any[]` when its resolved shape has
/// no `length` and no index signature. Anything surge resolves only in part
/// is left alone.
fn names_non_array_object_type(name: &str, ctx: &mut CheckerContext) -> bool {
    let Some(crate::symbols::TypeDeclarationInfo::Interface(info)) = ctx.lookup_type_declaration(name) else {
        return false;
    };
    if !info.body.type_parameters.is_empty() || !info.body.extends.is_empty() {
        return false;
    }
    let checkpoint = ctx.diagnostics().len();
    let reference = surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
        type_argument_spans: Vec::new(),
        name: name.to_string(),
        span: None,
        type_arguments: Vec::new(),
    }));
    let resolved = crate::infer::map_parsed_type(reference, ctx);
    ctx.truncate_diagnostics(checkpoint);
    // Only the type's own shape decides array-likeness; a library interface
    // (`Date`) degraded deep in its members is still no array.
    if resolved.is_unmodelled() {
        return false;
    }
    matches!(
        resolved.peeled(),
        surge_ts_types::Type::Object(object)
            if !object.properties.contains_key("length")
                && object.string_index_type.is_none()
                && object.number_index_type.is_none()
                && object.call_signature().is_none()
                && object.construct_signature().is_none()
    )
}

fn index_signature_key_reference_is_invalid(
    finding: &surge_ts_syntax::ParsedGrammarDiagnostic,
    ctx: &mut CheckerContext,
) -> bool {
    let Some([name, start, end]) = finding
        .name
        .as_deref()
        .map(|payload| payload.split('\0').collect::<Vec<_>>())
        .and_then(|parts| <[&str; 3]>::try_from(parts).ok())
    else {
        return false;
    };
    let (Ok(start), Ok(end)) = (start.parse::<usize>(), end.parse::<usize>()) else {
        return false;
    };
    let names_interface = matches!(
        ctx.lookup_type_declaration(name),
        Some(crate::symbols::TypeDeclarationInfo::Interface(_))
    );
    let reference = surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
        type_argument_spans: Vec::new(),
        name: name.to_string(),
        span: Some(surge_ts_syntax::TextSpan { start, end }),
        type_arguments: Vec::new(),
    }));
    let key = crate::infer::map_parsed_type(reference, ctx);
    names_interface
        || match key.peeled() {
            surge_ts_types::Type::ErrorType
            | surge_ts_types::Type::Boolean
            | surge_ts_types::Type::Array(_)
            | surge_ts_types::Type::Tuple(_)
            | surge_ts_types::Type::Function(_) => true,
            surge_ts_types::Type::Object(object) => !object.is_intersection,
            _ => false,
        }
}

/// Whether an alias written as `body` declares a union every member of which
/// is string- or number-like. `None` for a body whose declared type needs
/// resolving, or a union tsc would reduce.
fn declared_literal_union(body: &surge_ts_syntax::ParsedType) -> Option<bool> {
    use surge_ts_syntax::ParsedType as P;
    let is_settled = |ty: &P| {
        matches!(
            ty,
            P::String
                | P::Number
                | P::Boolean
                | P::BigInt
                | P::Symbol
                | P::StringLiteral(_)
                | P::NumberLiteral(_)
                | P::BooleanLiteral(_)
                | P::Object(_)
                | P::Function(_)
                | P::Array(_)
                | P::Tuple(_)
        )
    };
    let P::Union(members) = body else {
        return is_settled(body).then_some(false);
    };
    if !members.iter().all(is_settled) {
        return None;
    }
    let has = |pred: fn(&P) -> bool| members.iter().any(pred);
    let reduces = (has(|ty| matches!(ty, P::String)) && has(|ty| matches!(ty, P::StringLiteral(_))))
        || (has(|ty| matches!(ty, P::Number)) && has(|ty| matches!(ty, P::NumberLiteral(_))))
        || members
            .iter()
            .enumerate()
            .any(|(index, ty)| members[..index].contains(ty));
    if reduces {
        return None;
    }
    Some(members.iter().all(|ty| {
        matches!(ty, P::String | P::Number | P::StringLiteral(_) | P::NumberLiteral(_))
    }))
}

/// tsc's `isTypeNodeKind` over the keyword an identifier's text scans as.
fn is_type_keyword_name(name: &str) -> bool {
    matches!(
        name,
        "any"
            | "unknown"
            | "number"
            | "bigint"
            | "object"
            | "boolean"
            | "string"
            | "symbol"
            | "void"
            | "undefined"
            | "never"
            | "intrinsic"
    )
}

/// The parse failures oxc classified that the grammar pass does not already
/// report for this file. oxc numbers some of the same violations (TS1015,
/// TS1049) at its own span; the grammar pass anchors them where tsc does, so
/// a code it reports here is its to report and oxc's copy would be a second
/// diagnostic for one error.
pub(crate) fn unclaimed_parser_errors<'a>(
    errors: &'a [surge_ts_syntax::ParserError],
    findings: &[surge_ts_syntax::ParsedGrammarDiagnostic],
    ctx: &CheckerContext,
) -> impl Iterator<Item = &'a surge_ts_syntax::ParserError> {
    let claimed: Vec<u32> = findings
        .iter()
        .filter_map(|finding| match grammar_finding_diagnostic(finding, ctx)?.code {
            surge_ts_diagnostics::DiagnosticCode::TypeScript(number) => Some(number),
            surge_ts_diagnostics::DiagnosticCode::Custom(_) => None,
        })
        .collect();
    // tsc's `checkGrammarModifiers` returns at the first modifier it rejects,
    // so an `abstract` member outside an abstract class (TS1244/TS1253) never
    // reaches the repeated-modifier check oxc reports as TS1030.
    let abstract_members: Vec<surge_ts_syntax::TextSpan> = findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.kind,
                surge_ts_syntax::ParsedGrammarDiagnosticKind::AbstractMethodOutsideAbstractClass
                    | surge_ts_syntax::ParsedGrammarDiagnosticKind::AbstractPropertyOutsideAbstractClass
            )
        })
        .map(|finding| finding.span)
        .collect();
    let in_declaration_file = surge_ts_syntax::is_declaration_file_name(&ctx.file_name);
    // tsc's parser reports `super<T>` as TS2754 and still builds the
    // expression with its type arguments, so a `.member` after it is not the
    // instantiation-expression access oxc reports as TS1477.
    let super_type_arguments: Vec<usize> = findings
        .iter()
        .filter(|finding| matches!(finding.kind, surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(2754)))
        .map(|finding| finding.span.start)
        .collect();
    // tsc names the misplaced modifier's own error (TS1242 `abstract`,
    // TS1275 `accessor`, TS1433 on a `this` parameter) where oxc reports the
    // generic TS1090, and TS18041 for a `return` in a static block where oxc
    // reports TS1108.
    let spans_of = |codes: &[u32]| -> Vec<surge_ts_syntax::TextSpan> {
        findings
            .iter()
            .filter(|finding| {
                matches!(finding.kind, surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(code) if codes.contains(&code))
            })
            .map(|finding| finding.span)
            .collect()
    };
    let parameter_modifiers = spans_of(&[1242, 1275, 1433]);
    let static_block_returns = spans_of(&[18041]);
    let overlaps = |spans: &[surge_ts_syntax::TextSpan], span: surge_ts_syntax::TextSpan| {
        spans.iter().any(|finding| {
            (finding.start <= span.start && span.start < finding.end.max(finding.start + 1))
                || (span.start <= finding.start && finding.start < span.end.max(span.start + 1))
        })
    };
    errors.iter().filter(move |error| {
        if error.code.is_some_and(|code| claimed.contains(&code)) {
            return false;
        }
        // The grammar pass ports tsc's `checkAmbientInitializer` whole — which
        // oxc applies too broadly (a decorated `declare` field never reaches
        // it) — so where that pass runs, oxc's TS1039 is never the report. It
        // does not run over a declaration file.
        if error.code == Some(1039) && !in_declaration_file {
            return false;
        }
        // Likewise `checkGrammarModifiers`' decorator rules: oxc also raises
        // TS1249 for a decorated accessor without a body, which tsc reports as
        // TS1206, so the grammar pass's verdict is the only one.
        if error.code == Some(1249) && !in_declaration_file {
            return false;
        }
        if error.code == Some(1477)
            && error.span.is_some_and(|span| super_type_arguments.contains(&span.start))
        {
            return false;
        }
        if let Some(span) = error.span {
            if error.code == Some(1090) && overlaps(&parameter_modifiers, span) {
                return false;
            }
            if error.code == Some(1108) && overlaps(&static_block_returns, span) {
                return false;
            }
        }
        // oxc rejects any parameter-property modifier outside a constructor
        // as TS1090; tsc's modifier grammar rejects only `static`, `export`,
        // `declare` and `async` on a parameter and reports a misplaced
        // parameter property as TS2369 instead.
        if error.code == Some(1090)
            && error.span_text.as_deref().is_some_and(|modifier| {
                matches!(modifier, "public" | "private" | "protected" | "readonly" | "override")
            })
        {
            return false;
        }
        !(error.code == Some(1030)
            && error.span.is_some_and(|span| {
                abstract_members
                    .iter()
                    .any(|member| member.start <= span.start && span.end <= member.end)
            }))
    })
}

/// tsc's `checkExportAssignment` condition: `module` of ES2015 or later, not
/// `preserve`, and a file whose emit format is not CommonJS (an explicit
/// `.cts`/`.cjs`, or under node16/nodenext any file whose implied format is
/// not ESM).
fn export_assignment_targets_esm(ctx: &CheckerContext) -> bool {
    let module = ctx.options.module_emit;
    if module.is_node() {
        return ctx.options.esm_module_files.contains(ctx.file_name.as_str());
    }
    let lower = ctx.file_name.to_ascii_lowercase();
    module.is_ecmascript() && !lower.ends_with(".cts") && !lower.ends_with(".cjs")
}

/// tsc's `GetEmitModuleFormatOfFile` answering CommonJS: `module: commonjs`,
/// a node module kind for a file whose implied format is not ESM, or a
/// `.cts`/`.cjs` file under any other kind. A package.json `"type"` is only
/// read under node resolution, where `esm_module_files` records it.
pub(crate) fn file_emits_commonjs(ctx: &CheckerContext) -> bool {
    let module = ctx.options.module_emit;
    if module.is_node() {
        return !ctx.options.esm_module_files.contains(ctx.file_name.as_str());
    }
    let lower = ctx.file_name.to_ascii_lowercase();
    if lower.ends_with(".mts") || lower.ends_with(".mjs") {
        return false;
    }
    module == crate::ModuleEmitKind::CommonJS
        || lower.ends_with(".cts")
        || lower.ends_with(".cjs")
}

fn is_mts_or_cts_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    lower.ends_with(".mts") || lower.ends_with(".cts")
}

/// tsgo's module-kind switch in `checkGrammarAwaitOrAwaitUsing` and
/// `checkGrammarForInOrForOfStatement`: top-level `await` needs `module`
/// `es2022`, `esnext`, `preserve`, `system` or a node kind (where a
/// CommonJS-format file is TS1309 instead) and a target of ES2017 or later.
fn top_level_await_unsupported(ctx: &CheckerContext) -> bool {
    use crate::ModuleEmitKind as Module;
    let target_supported = ctx.options.language_version >= crate::LanguageVersion::ES2017;
    match ctx.options.module_emit {
        Module::Node16 | Module::Node18 | Module::Node20 | Module::NodeNext => {
            !target_supported
                && crate::modules::implied_format_for_emit(ctx, &ctx.file_name)
                    != Some(crate::modules::ModuleFormat::CommonJs)
        }
        Module::ES2022 | Module::ESNext | Module::Preserve => !target_supported,
        Module::CommonJS | Module::ES2015 | Module::ES2020 => true,
    }
}

fn grammar_finding_diagnostic(
    finding: &surge_ts_syntax::ParsedGrammarDiagnostic,
    ctx: &CheckerContext,
) -> Option<Diagnostic> {
    use surge_ts_syntax::ParsedGrammarDiagnosticKind as Kind;

    let diagnostic = match finding.kind {
        Kind::Ts(2683 | 7041) if !ctx.options.no_implicit_this => return None,
        Kind::Ts(7028) if ctx.options.allow_unused_labels != Some(false) => return None,
        Kind::Ts(7032) => {
            let payload = finding.name.as_deref()?;
            let (name, private) = match payload.split_once('\0') {
                Some((name, _)) => (name, true),
                None => (payload, false),
            };
            if !ctx.options.no_implicit_any || (private && super::file_classify::is_declaration_file_name(&ctx.file_name)) {
                return None;
            }
            Diagnostic::ts7032(name, ctx.file_name.clone())
        }
        // Answered by `emit_deferred_grammar_diagnostics`, once the file's type
        // declarations are in place.
        Kind::NamedSignatureParameterWithoutType
        | Kind::ComputedTypeMemberName
        | Kind::IndexSignatureKeyReference
        | Kind::AsyncReturnTypeReference
        | Kind::UnreachableAfterExhaustiveSwitch
        | Kind::RestParameterTypeReference => {
            return None;
        }
        Kind::Ts(1202) if !ctx.options.module_emit.is_ecmascript() => return None,
        Kind::Ts(1203) if !export_assignment_targets_esm(ctx) => return None,
        Kind::Ts(2699)
            if ctx.options.use_define_for_class_fields
                && !finding.name.as_deref().is_some_and(|name| name.starts_with("prototype\0")) =>
        {
            return None;
        }
        Kind::Ts(2301 | 2376 | 2401) if ctx.options.emit_standard_class_fields() => return None,
        // `errorSkippedOnNoEmit`: the CommonJS wrapper's names only collide
        // when the file is emitted.
        Kind::Ts(2441 | 1216) if ctx.options.no_emit || !file_emits_commonjs(ctx) => return None,
        Kind::Ts(2818) if ctx.options.no_emit || ctx.options.target_es2022 => return None,
        // `import string = N.T` is reported only when the target has a type
        // meaning; the finding carries the entity name to resolve.
        Kind::Ts(2438) => {
            let (name, entity) = finding.name.as_deref()?.split_once('\0')?;
            let entity: String = entity.chars().filter(|c| !c.is_whitespace()).collect();
            ctx.lookup_type_declaration(&entity)?;
            Diagnostic::ts2438(name, ctx.file_name.clone())
        }
        // `verbatimModuleSyntax` in a file emitted as CommonJS (TS1286 in a
        // `.cts`/`.cjs` file). A dynamic import is judged by `module:
        // commonjs` alone, and `checkGrammarImportCallExpression` stops there.
        Kind::Ts(1295) => {
            let applies = if finding.name.as_deref() == Some("import") {
                ctx.options.module_emit == crate::ModuleEmitKind::CommonJS
            } else {
                file_emits_commonjs(ctx)
            };
            if !ctx.options.verbatim_module_syntax || !applies {
                return None;
            }
            let lower = ctx.file_name.to_ascii_lowercase();
            if lower.ends_with(".cts") || lower.ends_with(".cjs") {
                Diagnostic::ts1286(ctx.file_name.clone())
            } else {
                Diagnostic::ts1295(ctx.file_name.clone())
            }
        }
        Kind::Ts(1323 | 1324 | 1325 | 1326)
            if ctx.options.verbatim_module_syntax
                && ctx.options.module_emit == crate::ModuleEmitKind::CommonJS =>
        {
            return None;
        }
        Kind::Ts(1009)
            if finding.name.as_deref() == Some("import")
                && ctx.options.verbatim_module_syntax
                && ctx.options.module_emit == crate::ModuleEmitKind::CommonJS =>
        {
            return None;
        }
        Kind::Ts(1287) if !ctx.options.verbatim_module_syntax || !file_emits_commonjs(ctx) => {
            return None;
        }
        Kind::Ts(2725) if !file_emits_commonjs(ctx) => return None,
        Kind::Ts(2725) => {
            Diagnostic::ts2725(ctx.options.module_emit.option_name(), ctx.file_name.clone())
        }
        // tsc reports unreachable code as an error only under an explicit
        // `allowUnreachableCode: false`; unset makes it a suggestion.
        Kind::Ts(7027) if !ctx.options.report_unreachable_code => return None,
        Kind::Ts(1280) => {
            if !ctx.options.isolated_modules && !ctx.options.verbatim_module_syntax {
                return None;
            }
            let flag = if ctx.options.verbatim_module_syntax { "verbatimModuleSyntax" } else { "isolatedModules" };
            Diagnostic::ts1280(flag, ctx.file_name.clone())
        }
        Kind::Ts(2565) if !(ctx.options.strict_null_checks && ctx.options.strict_property_initialization) => {
            return None;
        }
        Kind::Ts(2823) if ctx.options.module_emit.supports_import_attributes() => return None,
        Kind::Ts(7005 | 7031) if !ctx.options.no_implicit_any => return None,
        Kind::Ts(1323) if ctx.options.module_emit != crate::ModuleEmitKind::ES2015 => return None,
        // `checkGrammarImportCallExpression` stops at TS1323 under `module:
        // es2015`, and takes a second argument under the module kinds that
        // support import attributes (and `node16`).
        Kind::Ts(1324)
            if ctx.options.module_emit == crate::ModuleEmitKind::ES2015
                || ctx.options.module_emit.is_node()
                || matches!(
                    ctx.options.module_emit,
                    crate::ModuleEmitKind::ESNext | crate::ModuleEmitKind::Preserve
                ) =>
        {
            return None;
        }
        Kind::Ts(1325 | 1326) if ctx.options.module_emit == crate::ModuleEmitKind::ES2015 => {
            return None;
        }
        Kind::Ts(1009) if finding.name.as_deref() == Some("import") => {
            let module = ctx.options.module_emit;
            if module == crate::ModuleEmitKind::ES2015
                || module.is_node()
                || matches!(module, crate::ModuleEmitKind::ESNext | crate::ModuleEmitKind::Preserve)
            {
                return None;
            }
            Diagnostic::ts1009(ctx.file_name.clone())
        }
        Kind::Ts(7060) if !is_mts_or_cts_file(&ctx.file_name) => return None,
        Kind::Ts(1309)
            if !ctx.options.module_emit.is_node()
                || crate::modules::implied_format_for_emit(ctx, &ctx.file_name)
                    != Some(crate::modules::ModuleFormat::CommonJs) =>
        {
            return None;
        }
        // A file without an import or export that module detection still makes
        // a module (`IsEffectiveExternalModule`). A JavaScript file's CommonJS
        // use can too, which is not known here, so one is never reported.
        Kind::Ts(1375 | 1431 | 2853)
            if surge_ts_syntax::is_javascript_file_name(&ctx.file_name)
                || crate::program::file_is_forced_module(
                    &ctx.file_name,
                    ctx.file_name.to_ascii_lowercase().ends_with(".tsx"),
                    &ctx.options,
                ) =>
        {
            return None;
        }
        Kind::Ts(1378 | 1432 | 2854) if !top_level_await_unsupported(ctx) => return None,
        Kind::Ts(1470)
            if !ctx.options.module_emit.is_node()
                || crate::modules::implied_format_for_emit(ctx, &ctx.file_name)
                    == Some(crate::modules::ModuleFormat::Esm) =>
        {
            return None;
        }
        Kind::TsUnderStrictNullChecks(_) if !ctx.options.strict_null_checks => return None,
        Kind::TsWithoutStrictNullChecks(number)
            if ctx.options.strict_null_checks || (matches!(number, 7005 | 7018 | 7031) && !ctx.options.no_implicit_any) =>
        {
            return None;
        }
        Kind::TsUnderLegacyDecorators(_) if !ctx.options.experimental_decorators => return None,
        Kind::TsUnderEsDecorators(_) if ctx.options.experimental_decorators => return None,
        Kind::TsUnlessStandardClassFields(_) if ctx.options.emit_standard_class_fields() => return None,
        Kind::Ts(number)
        | Kind::TsUnderStrictNullChecks(number)
        | Kind::TsWithoutStrictNullChecks(number)
        | Kind::TsUnderLegacyDecorators(number)
        | Kind::TsUnderEsDecorators(number)
        | Kind::TsUnlessStandardClassFields(number) => {
            let args: Vec<surge_ts_diagnostics::DiagnosticArg> = finding
                .name
                .as_deref()
                .map(|names| names.split('\0').map(Into::into).collect())
                .unwrap_or_default();
            let descriptor = surge_ts_diagnostics::emitted_descriptor_for_number_with_arity(
                number,
                args.len(),
            )?;
            Diagnostic::from_descriptor(descriptor, args, ctx.file_name.clone())
        }
        Kind::ConstNotInitialized => Diagnostic::ts1155("const", ctx.file_name.clone()),
        Kind::AwaitOutsideAsyncFunction => Diagnostic::ts1308(ctx.file_name.clone()),
        Kind::ExportDeclarationInNamespace => Diagnostic::ts1194(ctx.file_name.clone()),
        Kind::DeleteOnIdentifierInStrictMode => Diagnostic::ts1102(ctx.file_name.clone()),
        Kind::EnumForwardReference => Diagnostic::ts2651(ctx.file_name.clone()),
        Kind::EnumMemberInitializerRequired => Diagnostic::ts1061(ctx.file_name.clone()),
        Kind::ConstEnumInitializerNotConstant => Diagnostic::ts2474(ctx.file_name.clone()),
        Kind::AmbientEnumInitializerNotConstant => Diagnostic::ts1066(ctx.file_name.clone()),
        Kind::DuplicateObjectLiteralProperty => Diagnostic::ts1117(ctx.file_name.clone()),
        Kind::FunctionImplementationMissing => Diagnostic::ts2391(ctx.file_name.clone()),
        Kind::ConstructorImplementationMissing => Diagnostic::ts2390(ctx.file_name.clone()),
        Kind::MultipleDefaultExports => Diagnostic::ts2528(ctx.file_name.clone()),
        Kind::ImplicitAnyMember => {
            if !ctx.options.no_implicit_any {
                return None;
            }
            let Some(name) = finding.name.as_deref() else {
                return None;
            };
            Diagnostic::ts7008(name, "any", ctx.file_name.clone())
        }
        Kind::AsyncReturnTypeNotPromise => {
            let Some(written) = finding.name.as_deref() else {
                return None;
            };
            Diagnostic::ts1064(written, ctx.file_name.clone())
        }
        Kind::RequiredTypeParameterAfterOptional => Diagnostic::ts2706(ctx.file_name.clone()),
        Kind::CircularTypeAlias => {
            let Some(name) = finding.name.as_deref() else {
                return None;
            };
            Diagnostic::ts2456(name, ctx.file_name.clone())
        }
        Kind::OptionalParameterWithInitializer => Diagnostic::ts1015(ctx.file_name.clone()),
        Kind::RequiredParameterAfterOptional => Diagnostic::ts1016(ctx.file_name.clone()),
        Kind::AmbientInitializer => Diagnostic::ts1039(ctx.file_name.clone()),
        Kind::AmbientConstInitializer => Diagnostic::ts1254(ctx.file_name.clone()),
        Kind::SetAccessorParameterCount => Diagnostic::ts1049(ctx.file_name.clone()),
        Kind::GetAccessorWithoutReturn => Diagnostic::ts2378(ctx.file_name.clone()),
        Kind::PropertyAccessorOverride
        | Kind::AccessorPropertyOverride
        | Kind::MethodAccessorOverride
        | Kind::PropertyMethodOverride
        | Kind::AccessorMethodOverride => {
            let Some([member, base, derived]) = finding
                .name
                .as_deref()
                .map(|names| names.split('\0').collect::<Vec<_>>())
                .and_then(|names| <[&str; 3]>::try_from(names).ok())
            else {
                return None;
            };
            let file_name = ctx.file_name.clone();
            match finding.kind {
                Kind::PropertyAccessorOverride => Diagnostic::ts2610(member, base, derived, file_name),
                Kind::AccessorPropertyOverride => Diagnostic::ts2611(member, base, derived, file_name),
                Kind::MethodAccessorOverride => Diagnostic::ts2423(base, member, derived, file_name),
                Kind::PropertyMethodOverride => Diagnostic::ts2425(base, member, derived, file_name),
                _ => Diagnostic::ts2426(base, member, derived, file_name),
            }
        }
        Kind::ThisBeforeSuperCall => Diagnostic::ts17009(ctx.file_name.clone()),
        Kind::SuperPropertyBeforeSuperCall => Diagnostic::ts17011(ctx.file_name.clone()),
        Kind::GetAccessorLessAccessible => Diagnostic::ts2808(ctx.file_name.clone()),
        Kind::AccessorAbstractMismatch => Diagnostic::ts2676(ctx.file_name.clone()),
        Kind::SetAccessorReturnType => Diagnostic::ts1095(ctx.file_name.clone()),
        Kind::ObjectLiteralPropertyAndAccessor => Diagnostic::ts1119(ctx.file_name.clone()),
        Kind::AbstractMethodOutsideAbstractClass => Diagnostic::ts1244(ctx.file_name.clone()),
        Kind::AbstractPropertyOutsideAbstractClass => Diagnostic::ts1253(ctx.file_name.clone()),
        Kind::ParameterInitializerOutsideImplementation => {
            Diagnostic::ts2371(ctx.file_name.clone())
        }
        Kind::DuplicateMember => {
            let Some(name) = finding.name.as_deref() else {
                return None;
            };
            Diagnostic::ts2300(name, ctx.file_name.clone())
        }
        Kind::DuplicateImplementation => Diagnostic::ts2393(ctx.file_name.clone()),
        Kind::MultipleConstructorImplementations => Diagnostic::ts2392(ctx.file_name.clone()),
        Kind::MissingSuperCall => Diagnostic::ts2377(ctx.file_name.clone()),
        // `checkBinaryLikeExpression` reports it only when unreachable code is
        // not explicitly allowed.
        Kind::UnusedCommaOperand if ctx.options.allow_unreachable_code => return None,
        Kind::UnusedCommaOperand => Diagnostic::ts2695(ctx.file_name.clone()),
        Kind::AlwaysTruthyExpression => Diagnostic::ts2872(ctx.file_name.clone()),
        Kind::AlwaysFalsyExpression => Diagnostic::ts2873(ctx.file_name.clone()),
        Kind::NeverNullishCoalesceOperand => Diagnostic::ts2869(ctx.file_name.clone()),
        Kind::AlwaysNullishCoalesceOperand => Diagnostic::ts2871(ctx.file_name.clone()),
        Kind::ImplicitAnyConstructReturn | Kind::ImplicitAnyCallReturn | Kind::ImplicitAnySignatureParameter
            if !ctx.options.no_implicit_any =>
        {
            return None;
        }
        Kind::ImplicitAnyConstructReturn => Diagnostic::ts7013(ctx.file_name.clone()),
        Kind::ImplicitAnyCallReturn => Diagnostic::ts7020(ctx.file_name.clone()),
        Kind::ImplicitAnySignatureParameter => {
            let Some(name) = finding.name.as_deref() else {
                return None;
            };
            Diagnostic::ts7006(name, ctx.file_name.clone())
        }
        Kind::ImplicitAnyOverloadReturn => {
            if !ctx.options.no_implicit_any {
                return None;
            }
            Diagnostic::ts7012("any", ctx.file_name.clone())
        }
        Kind::ImplicitAnyReturn => {
            if !ctx.options.no_implicit_any {
                return None;
            }
            let Some(name) = finding.name.as_deref() else {
                return None;
            };
            Diagnostic::ts7010(name, "any", ctx.file_name.clone())
        }
    };

    Some(diagnostic.with_span(crate::context::convert_span(finding.span)))
}

/// tsc's `checkImportAttributes`: a `with { … }` clause, typed as an object
/// literal of its values' types, must be assignable to the global
/// `ImportAttributes` or `undefined`.
fn check_import_attributes(clauses: &[surge_ts_syntax::ParsedImportAttributes], ctx: &mut CheckerContext) {
    if clauses.is_empty() || ctx.lookup_type_declaration("ImportAttributes").is_none() {
        return;
    }
    let import_attributes = crate::infer::map_parsed_type(
        surge_ts_syntax::ParsedType::Named(std::sync::Arc::new(surge_ts_syntax::ParsedNamedType {
            type_argument_spans: Vec::new(),
            name: "ImportAttributes".to_string(),
            span: None,
            type_arguments: Vec::new(),
        })),
        ctx,
    );
    let target = surge_ts_types::union_type(vec![import_attributes, surge_ts_types::Type::Undefined]);
    let symbols = ctx.symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
    'clauses: for clause in clauses {
        let mut properties = surge_ts_types::PropertyMap::default();
        for (name, value) in &clause.attributes {
            let reported = ctx.diagnostics().len();
            let value_type = crate::infer::infer_expression(value, &symbols, ctx);
            ctx.truncate_diagnostics(reported);
            let crate::infer::InferredExpression::Known(ty) = value_type else {
                continue 'clauses;
            };
            properties.insert(
                std::sync::Arc::from(name.as_str()),
                surge_ts_types::ObjectProperty {
                    ty,
                    optional: false,
                    method: false,
                    readonly: false,
                    restriction: None,
                    index_slot: false,
                },
            );
        }
        let source = surge_ts_types::Type::Object(crate::metrics::alloc_object_type(properties, None));
        crate::checks::var::report_initializer_mismatch(&source, &target, Some(clause.span), ctx);
    }
}

pub(crate) fn check_program_file(
    file_index: usize,
    parsed_file: &ParsedProgramFile,
    shared_state: &ProgramCheckSharedState,
    ctx: &mut CheckerContext,
    timings: Option<&Arc<Mutex<ProgramTimings>>>,
) -> FileCheckResult {
    ctx.begin_file_check(parsed_file.file_name.clone());
    ctx.file_const_class_names.extend(parsed_file.statements.iter().filter_map(|statement| match statement {
        surge_ts_syntax::ParsedStatement::ClassDeclaration(class) if class.const_binding => {
            Some(std::sync::Arc::from(class.name.as_str()))
        }
        _ => None,
    }));

    if ctx.options.skip_lib_check && parsed_file.file_kind.is_declaration() {
        return FileCheckResult {
            file_index,
            diagnostics: Vec::new(),
            stats: CompatibilityStats::default(),
        };
    }

    // tsc's `checkSourceFile` checks a declaration file like any other unless
    // `skipLibCheck` skips it. A package's, a generated one's and a default
    // library's report only what binding them does.
    if parsed_file.file_kind.is_declaration() && parsed_file.file_kind != FileKind::RootDeclaration {
        emit_bind_diagnostics(parsed_file, ctx);
        emit_grammar_diagnostics(&parsed_file.grammar_diagnostics, ctx);
        emit_unsupported_declaration_diagnostics(&parsed_file.statements, ctx);
        let mut diagnostics = std::mem::take(&mut ctx.diagnostics);
        reconcile_bind_diagnostics(&mut diagnostics, parsed_file);
        let stats = std::mem::take(&mut ctx.stats);
        return FileCheckResult {
            file_index,
            diagnostics,
            stats,
        };
    }

    let ambient_file;
    let parsed_file = if parsed_file.file_kind == FileKind::RootDeclaration {
        let mut file = parsed_file.clone();
        super::make_declaration_file_ambient(&mut file.statements);
        ambient_file = file;
        &ambient_file
    } else {
        parsed_file
    };

    emit_bind_diagnostics(parsed_file, ctx);
    let script_namespace = surge_ts_syntax::ParsedGrammarDiagnosticKind::Ts(1280);
    if parsed_file.is_module && parsed_file.grammar_diagnostics.iter().any(|finding| finding.kind == script_namespace) {
        // The walker judges a script by its text; `moduleDetection` may make
        // the file a module all the same.
        let findings: Vec<_> = parsed_file
            .grammar_diagnostics
            .iter()
            .filter(|finding| finding.kind != script_namespace)
            .cloned()
            .collect();
        emit_grammar_diagnostics(&findings, ctx);
    } else {
        emit_grammar_diagnostics(&parsed_file.grammar_diagnostics, ctx);
    }
    if surge_ts_syntax::is_javascript_file_name(&parsed_file.file_name) {
        ctx.javascript_expando_objects = Arc::new(javascript_expando_containers(&parsed_file.statements));
    }
    ctx.parenthesized_expressions = parsed_file.parenthesized_expressions.clone();
    if !parsed_file.is_module
        && !crate::program::file_is_forced_module(
            &parsed_file.file_name,
            parsed_file.jsx_factory_uses.first_tag.is_some(),
            &ctx.options,
        )
    {
        ctx.global_this_starts = parsed_file.global_this_starts.clone();
    } else {
        ctx.module_this_starts = parsed_file.global_this_starts.clone();
    }
    ctx.literal_this_members = parsed_file.literal_this_members.clone();
    ctx.let_assignments = parsed_file.let_assignments.clone();
    ctx.jsx_factory_uses = parsed_file.jsx_factory_uses.clone();
    crate::checks::jsx::check_jsx_runtime_import(ctx);
    for diagnostic in shared_state.emit_helper_diagnostics.get(file_index).into_iter().flatten() {
        ctx.push(diagnostic.clone());
    }

    if parsed_file.is_module {
        let Some(module_analysis) = shared_state.module_analyses[file_index].as_ref() else {
            return FileCheckResult {
                file_index,
                diagnostics: Vec::new(),
                stats: CompatibilityStats::default(),
            };
        };

        let imported_bindings = shared_state.module_import_bindings[file_index].as_ref();

        let module_resolution_scope = shared_state.module_resolution_scopes[file_index]
            .as_ref()
            .cloned()
            .unwrap_or_else(|| {
                let mut layers = vec![module_analysis.local_type_declarations.clone()];
                if let Some(imported_bindings) = imported_bindings {
                    layers.extend(imported_bindings.scope_layers());
                }
                record_module_scope_cache_miss();
                Arc::new(TypeDeclarationScope::new(layers))
            });
        if shared_state.module_resolution_scopes[file_index].is_some() {
            record_module_scope_cache_hit();
        }

        // The ambient globals (~4k entries on @types/node projects) are a
        // read-only lookup backdrop for the file check; holding them as a
        // `parent` fallback keeps the per-file working set O(imports + locals)
        // where a clone-then-insert deep-copied every global per file.
        let ambient_globals = Arc::new(
            ctx.ambient_global_symbols
                .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
        );
        // The scripts' globals sit behind the module's own scope, which is a
        // declaration space of its own: a module-level `let` never redeclares a
        // script's.
        let globals_parent = match shared_state.module_script_globals.as_ref() {
            Some(script_globals) => Arc::new(crate::symbols::SymbolTable::declaration_scope(
                Arc::new(
                    script_globals
                        .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext)
                        .with_parent_fallback(ambient_globals),
                ),
            )),
            None => ambient_globals,
        };
        let mut merged_symbols =
            crate::symbols::SymbolTable::file_check_root(globals_parent.clone());
        if let Some(imported_bindings) = imported_bindings {
            for (name, symbol) in imported_bindings.symbols.iter_shared() {
                // A value its module could not type before binding is typed on
                // this first read (`LazyInitializerValue`).
                let symbol = if crate::modules::is_lazy_initializer(&symbol.ty) {
                    std::sync::Arc::new(crate::symbols::SymbolInfo {
                        ty: crate::checks::function::settle_lazy_read(symbol.ty.clone()),
                        kind: symbol.kind,
                        function_signature: symbol.function_signature.clone(),
                    })
                } else {
                    symbol.clone()
                };
                let _ = merged_symbols.insert_shared(name.clone(), symbol);
            }
        }
        for (name, symbol) in module_analysis.local_symbols.iter_shared() {
            let _ = merged_symbols.insert_shared(name.clone(), symbol.clone());
        }

        ctx.type_declarations = module_analysis.local_type_declarations.as_ref().clone();
        ctx.type_declaration_scope = Some(module_resolution_scope);
        ctx.set_symbols(
            merged_symbols.clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext),
        );

        // A module-scope declaration of the same name shadows the import (and is
        // its own duplicate-identifier error), so it is excluded rather than
        // reported as a type-only value use.
        let module_declared =
            crate::program::ambient::module_scope_own_declared_names(&parsed_file.statements);
        ctx.set_file_type_only_import_names(
            crate::program::ambient::type_only_import_bound_names(&parsed_file.statements)
                .into_iter()
                .filter(|name| !module_declared.contains(name)),
        );
        ctx.set_file_type_only_alias_names(
            imported_bindings
                .into_iter()
                .flat_map(|bindings| bindings.type_only_aliases.iter().cloned())
                .filter(|(name, _)| !module_declared.contains(name.as_ref())),
        );
        ctx.file_type_only_import_handles = imported_bindings
            .into_iter()
            .flat_map(|bindings| bindings.symbols.iter_shared())
            .filter_map(|(name, _)| Some((name.clone(), ctx.symbols.get_own_handle(name)?)))
            .collect();
        ctx.set_file_complete_namespace_import_names(
            imported_bindings
                .into_iter()
                .flat_map(|bindings| bindings.complete_namespace_imports.iter())
                .filter(|name| !module_declared.contains(name.as_ref())),
        );
        ctx.set_file_import_names(
            crate::program::ambient::import_bound_names(&parsed_file.statements)
                .into_iter()
                .filter(|name| !module_declared.contains(name)),
            crate::program::ambient::namespace_import_names(&parsed_file.statements)
                .into_iter()
                .filter(|name| !module_declared.contains(name)),
        );

        if !ctx.umd_global_names.is_empty() {
            let declared = module_scope_declared_names(&parsed_file.statements);
            ctx.set_file_umd_global_names(true, |name| {
                declared.contains(name)
                    || module_analysis.local_symbols.get_handle(name).is_some()
                    || module_analysis.local_type_declarations.get(name).is_some()
                    || imported_bindings.is_some_and(|bindings| bindings.binds_name(name))
            });
        }

        let current_type_declarations = ctx.type_declarations.clone();
        let current_symbols = ctx
            .symbols
            .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
        let validation_symbols = crate::modules::collect_exportable_value_symbols(
            &parsed_file.statements,
            &current_type_declarations,
            &current_symbols,
            None,
            parsed_file.is_module,
            ctx,
        );
        // The collection backs its table with the lib's globals alone; the
        // function bodies that fall back to it read the scripts' globals too.
        let validation_symbols = if shared_state.module_script_globals.is_some() {
            validation_symbols.with_parent_fallback(globals_parent.clone())
        } else {
            validation_symbols
        };
        let saved_symbols = std::mem::replace(&mut ctx.symbols, validation_symbols);

        let validation_start = Instant::now();
        validate_local_type_declarations(&parsed_file.statements, &parsed_file.file_name, ctx);
        let validation_duration = validation_start.elapsed();
        record_program_timing(timings, |timings| {
            timings.declaration_validation += validation_duration
        });
        record_program_file_timing(timings, &parsed_file.file_name, |metrics| {
            metrics.validate_local_type_declarations_passes += 1;
            metrics.validate_local_type_declarations_items +=
                count_local_type_declarations_in_statements(&parsed_file.statements) as u64;
            metrics.validate_local_type_declarations_duration += validation_duration;
        });

        let utility_validation_start = Instant::now();
        validate_direct_utility_aliases(&parsed_file.statements, ctx);
        record_program_timing(timings, |timings| {
            timings.utility_alias_validation += utility_validation_start.elapsed()
        });

        let validation_symbols =
            std::sync::Arc::new(std::mem::replace(&mut ctx.symbols, saved_symbols));

        let mut signature_ctx = ctx.clone_without_diagnostics();
        // A class's constructor parameters are mapped against the context, not
        // the signature scope, so a default naming a module value reads it here.
        signature_ctx.module_value_fallback = Some(validation_symbols.clone());
        signature_ctx.reset_utility_diagnostic_keys();
        signature_ctx.resolved_named_types =
            std::sync::Arc::new(std::sync::Mutex::new(Default::default()));
        // Seed from `validation_symbols` rather than `merged_symbols`: it carries
        // the local `const`/`let`/`var` value symbols inferred during declaration
        // validation, so `typeof <localConst>` resolves inside parameter type
        // annotations (function signatures see them, not just type aliases).
        //
        // Only the file's own top-level function declarations are skipped (they
        // are re-registered below, and re-seeding them would trip the duplicate
        // signature check). Imported and global functions are kept so that
        // `typeof <importedFn>` in a parameter annotation still resolves.
        let mut signature_local_symbols = crate::symbols::SymbolTable::new();
        for (name, symbol) in validation_symbols.iter_shared() {
            let is_local_function_declaration =
                matches!(symbol.kind, crate::symbols::SymbolKind::Function)
                    && module_analysis.local_symbols.get(name).is_some();
            if !is_local_function_declaration {
                signature_local_symbols.insert_shared(name.clone(), symbol.clone());
            }
        }
        // `validation_symbols` reaches globals through its parent fallback, so
        // the loop above seeds only file-level names; re-attach the globals so
        // `typeof <globalFn>` in a parameter annotation still resolves. Local
        // function declarations stay invisible (globals do not contain them),
        // preserving the duplicate-signature exclusion.
        let mut signature_local_symbols =
            signature_local_symbols.with_parent_fallback(globals_parent.clone());
        let mut final_function_signatures = HashMap::new();
        collect_function_signatures_from_statements(
            &parsed_file.statements,
            file_index,
            &mut signature_local_symbols,
            &mut final_function_signatures,
            &mut signature_ctx,
        );
        extend_diagnostics_dedup(&mut ctx.diagnostics, signature_ctx.diagnostics);

        ctx.module_value_fallback = Some(validation_symbols);

        let statement_check_start = Instant::now();
        crate::flow::begin_never_initialized_file(
            &parsed_file.statements,
            parsed_file.is_module,
            &parsed_file.definite_writes,
            ctx,
        );
        ctx.namespace_require_reads =
            super::namespace_require_reads(&parsed_file.statements, &parsed_file.module_reads);
        crate::semantic::capture_file_scope(ctx, file_index, &parsed_file.statements, &final_function_signatures);
        check_program_file_statements(
            &parsed_file.statements,
            file_index,
            &final_function_signatures,
            ctx,
        );
        check_import_attributes(&parsed_file.import_attributes, ctx);
        ctx.namespace_require_reads = None;
        ctx.module_value_fallback = None;

        record_program_timing(timings, |timings| {
            timings.per_file_statement_checking += statement_check_start.elapsed()
        });
    } else {
        // Clone the prebuilt global+ambient table rather than rebuilding it
        // per worker, so every script file sees the same merged globals.
        ctx.type_declarations = clone_type_declaration_table(
            &shared_state.script_type_declarations,
            timings,
            TableCloneKind::General,
        );
        ctx.type_declaration_scope = None;

        let mut script_sym = shared_state
            .global_symbols
            .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
        for (name, symbol) in ctx.ambient_global_symbols.iter_handles() {
            let _ = script_sym.insert_handle(name.clone(), clone_symbol_info_handle(symbol));
        }
        // Other scripts' values are globals here too. A `var` this file declares
        // itself is the first declaration of it only against later files, and
        // TS2403 compares every redeclaration with the first one.
        let own_vars: std::collections::HashSet<&str> = parsed_file
            .statements
            .iter()
            .filter_map(|statement| match statement {
                surge_ts_syntax::ParsedStatement::VariableDeclaration(variable)
                    if matches!(variable.kind, surge_ts_syntax::ParsedVariableKind::Var) =>
                {
                    Some(variable.name.as_str())
                }
                _ => None,
            })
            .collect();
        for (other_index, values) in shared_state.script_values.iter().enumerate() {
            let Some(values) = values.as_ref().filter(|_| other_index != file_index) else {
                continue;
            };
            for (name, symbol) in values.iter_handles() {
                if other_index > file_index && own_vars.contains(name.as_ref()) {
                    continue;
                }
                if script_sym.get(name).is_none() {
                    let _ = script_sym.insert_handle(name.clone(), clone_symbol_info_handle(symbol));
                }
            }
        }
        ctx.set_symbols(script_sym);
        // A script imports nothing, but its per-file import sets must still
        // describe it: the qualified-name checks only judge a file they do.
        ctx.set_file_type_only_import_names(std::iter::empty());
        ctx.set_file_complete_namespace_import_names(std::iter::empty());
        ctx.set_file_import_names(
            crate::program::ambient::import_bound_names(&parsed_file.statements),
            crate::program::ambient::namespace_import_names(&parsed_file.statements),
        );

        let current_type_declarations = ctx.type_declarations.clone();
        let current_symbols = ctx
            .symbols
            .clone_with_reason(surge_ts_types::TypeCopyReason::ScopeOrContext);
        let validation_symbols = crate::modules::collect_exportable_value_symbols(
            &parsed_file.statements,
            &current_type_declarations,
            &current_symbols,
            None,
            parsed_file.is_module,
            ctx,
        );
        let saved_symbols = std::mem::replace(&mut ctx.symbols, validation_symbols);

        let validation_start = Instant::now();
        validate_local_type_declarations(&parsed_file.statements, &parsed_file.file_name, ctx);
        let validation_duration = validation_start.elapsed();
        record_program_timing(timings, |timings| {
            timings.declaration_validation += validation_duration
        });
        record_program_file_timing(timings, &parsed_file.file_name, |metrics| {
            metrics.validate_local_type_declarations_passes += 1;
            metrics.validate_local_type_declarations_items +=
                count_local_type_declarations_in_statements(&parsed_file.statements) as u64;
            metrics.validate_local_type_declarations_duration += validation_duration;
        });

        let utility_validation_start = Instant::now();
        validate_direct_utility_aliases(&parsed_file.statements, ctx);
        record_program_timing(timings, |timings| {
            timings.utility_alias_validation += utility_validation_start.elapsed()
        });

        let validation_symbols =
            std::sync::Arc::new(std::mem::replace(&mut ctx.symbols, saved_symbols));

        ctx.module_value_fallback = Some(validation_symbols.clone());
        let recollected_signatures = (!parsed_file.file_kind.is_declaration()
            && !parsed_file.import_call_specifiers.is_empty())
        .then(|| {
            super::recollect_script_function_signatures(
                &parsed_file.statements,
                file_index,
                validation_symbols,
                ctx,
            )
        });
        let function_signatures = recollected_signatures
            .as_ref()
            .unwrap_or(&shared_state.function_signatures);

        let statement_check_start = Instant::now();
        crate::flow::begin_never_initialized_file(
            &parsed_file.statements,
            parsed_file.is_module,
            &parsed_file.definite_writes,
            ctx,
        );
        ctx.namespace_require_reads =
            super::namespace_require_reads(&parsed_file.statements, &parsed_file.module_reads);
        crate::semantic::capture_file_scope(
            ctx,
            file_index,
            &parsed_file.statements,
            function_signatures,
        );
        check_program_file_statements(
            &parsed_file.statements,
            file_index,
            function_signatures,
            ctx,
        );
        ctx.namespace_require_reads = None;
        ctx.module_value_fallback = None;
        record_program_timing(timings, |timings| {
            timings.per_file_statement_checking += statement_check_start.elapsed()
        });
    }

    emit_deferred_grammar_diagnostics(ctx);
    let mut diagnostics = std::mem::take(&mut ctx.diagnostics);
    reconcile_bind_diagnostics(&mut diagnostics, parsed_file);
    apply_comment_directives(
        &mut diagnostics,
        &parsed_file.comment_directives,
        !parsed_file.parser_errors.is_empty(),
        &parsed_file.file_name,
    );
    let stats = std::mem::take(&mut ctx.stats);

    FileCheckResult {
        file_index,
        diagnostics,
        stats,
    }
}

/// The module-level variables of a JavaScript file whose initializer makes a
/// member write declare a member (`IsExpandoInitializer`): a function, or an
/// empty object literal with no annotation. The binder looks the receiver up
/// in the file's own scope only, so another file's variable is none of them.
fn javascript_expando_containers(statements: &[surge_ts_syntax::ParsedStatement]) -> std::collections::HashSet<String> {
    statements
        .iter()
        .map(|statement| match statement {
            surge_ts_syntax::ParsedStatement::ExportDeclaration(export) => match export.as_ref() {
                surge_ts_syntax::ParsedExportDeclaration::Statement { declaration, .. } => declaration.as_ref(),
                _ => statement,
            },
            _ => statement,
        })
        .filter_map(|statement| match statement {
            surge_ts_syntax::ParsedStatement::VariableDeclaration(variable)
                if variable.declared_type.is_none()
                    && match &variable.initializer {
                        Some(surge_ts_syntax::ParsedExpression::ObjectLiteral { properties, .. }) => {
                            properties.is_empty()
                        }
                        Some(surge_ts_syntax::ParsedExpression::ArrowFunction(_)) => true,
                        _ => false,
                    } =>
            {
                Some(variable.name.clone())
            }
            _ => None,
        })
        .collect()
}
