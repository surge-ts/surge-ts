
use surge_ts_types::fx::{FxHashMap, FxHashSet};
use super::{FileCacheLog, LiveCacheHandles};

#[derive(Debug, Default)]
pub(crate) struct StcCommitStats {
    pub(crate) files: usize,
    pub(crate) clean_commits: usize,
    pub(crate) miss_conflicts: usize,
    pub(crate) dependency_conflicts: usize,
    pub(crate) published_entries: usize,
    pub(crate) merge_skipped_existing: usize,
    pub(crate) merge_cap_blocked: usize,
}

pub(crate) enum CommitVerdict {
    Clean,
    /// The file observed a miss on a key an earlier-ordered file published; its
    /// speculative result may differ from serial and must be recomputed.
    MissConflict,
    /// The file consumed a worker-overlay entry inserted by a file that itself
    /// failed validation (or whose publication was incomplete).
    DependencyConflict,
}

/// Predicts which positions will conflict during the commit walk, as a
/// scheduling hint for pipelined replay. Over-prediction only wastes a replay
/// and under-prediction only triggers an inline recompute, so correctness never
/// depends on this — it merely decides which positions the pool pre-replays.
///
/// A position likely conflicts if a key it observed missing was inserted by an
/// earlier position, or it consumed the worker overlay of an earlier position
/// that is itself predicted to conflict. Dispatch is ascending per worker, so a
/// position's overlay producers are always earlier and already classified.
pub(crate) fn predict_conflicts(logs: &[Option<FileCacheLog>]) -> Vec<bool> {
    let mut predicted = vec![false; logs.len()];
    let mut earlier_inserts: FxHashSet<u64> = FxHashSet::default();
    for (index, log) in logs.iter().enumerate() {
        let Some(log) = log else { continue };
        let miss_conflict = !log.misses.is_disjoint(&earlier_inserts);
        let dep_conflict = log.overlay_deps.iter().any(|&file| predicted[file]);
        predicted[index] = miss_conflict || dep_conflict;
        for (_, _, digest) in &log.generic_inserts {
            earlier_inserts.insert(*digest);
        }
        for (_, _, digest) in &log.instantiation_inserts {
            earlier_inserts.insert(*digest);
        }
        for (_, _, digest) in &log.physical_inserts {
            earlier_inserts.insert(*digest);
        }
        for (_, _, digest) in &log.template_inserts {
            earlier_inserts.insert(*digest);
        }
        for (_, _, digest) in &log.method_inserts {
            earlier_inserts.insert(*digest);
        }
        for (_, _, digest) in &log.overload_inserts {
            earlier_inserts.insert(*digest);
        }
    }
    predicted
}

/// Dependency-driven submit schedule for the replay pipeline: for each
/// predicted-conflict position, the frontier index at which its replay should
/// start (`usize::MAX` for positions that are not pre-replayed).
///
/// A conflict `k` reads the committed store correctly only once every position
/// that publishes a key `k` observed missing has committed. The last such
/// publisher (by first-writer position, since first-writer-wins) is `k`'s
/// binding dependency; launching `k`'s replay the moment that publisher's
/// position finalizes means the replay reads a committed view already containing
/// all of `k`'s dependencies, so it does not over-recurse against a stale view
/// and validates on the first try. A conflict with no earlier publisher among
/// its misses is submittable from the start (index 0).
///
/// This only schedules; `commit_position` still validates and falls back to an
/// inline recompute, so an imprecise schedule costs at most a wasted replay or
/// an inline recompute, never correctness.
pub(crate) fn compute_submit_schedule(logs: &[Option<FileCacheLog>]) -> Vec<usize> {
    let predicted = predict_conflicts(logs);
    let n = logs.len();
    let mut submit_at = vec![usize::MAX; n];
    // First position (in serial order) that inserts each digest — the publisher
    // whose commit makes that key visible — and whether that publisher is itself
    // a predicted conflict.
    let mut first_writer: FxHashMap<u64, usize> = FxHashMap::default();
    for (index, log) in logs.iter().enumerate() {
        let Some(log) = log else { continue };
        if predicted[index] {
            let mut latest_dep: Option<usize> = None;
            // Pre-replay only conflicts whose every dependency is a *clean* file.
            // A replay reads the committed store rather than the worker's fan-out
            // snapshot, so its exact dependency set can differ from the worker
            // log; when a dependency is another conflict, that imprecision makes
            // the replay prone to staleness (the conflict's eventual committed
            // inserts need not match its worker log), and a stale replay
            // over-recurses expensively for nothing. Clean-dependent conflicts,
            // by contrast, depend only on files that commit deterministically and
            // early, so their replay reads a complete-enough view and validates.
            // Conflict-dependent positions fall to the inline recheck.
            let mut depends_on_conflict = false;
            for digest in &log.misses {
                if let Some(&producer) = first_writer.get(digest)
                    && producer < index
                {
                    latest_dep = Some(latest_dep.map_or(producer, |cur| cur.max(producer)));
                    if predicted[producer] {
                        depends_on_conflict = true;
                    }
                }
            }
            // A worker overlay-hit means the replay (which has no overlay) will
            // re-read that key from the committed store, so it depends on the
            // overlay producer having committed.
            for &producer in &log.overlay_deps {
                if producer < index {
                    latest_dep = Some(latest_dep.map_or(producer, |cur| cur.max(producer)));
                    if predicted[producer] {
                        depends_on_conflict = true;
                    }
                }
            }
            if !depends_on_conflict {
                // Submit after the last dependency's position finalizes (index
                // `producer + 1`); no dependency ⇒ submittable immediately.
                submit_at[index] = latest_dep.map_or(0, |producer| producer + 1);
            }
        }
        let mut record = |digest: u64| {
            first_writer.entry(digest).or_insert(index);
        };
        for (_, _, d) in &log.generic_inserts {
            record(*d);
        }
        for (_, _, d) in &log.instantiation_inserts {
            record(*d);
        }
        for (_, _, d) in &log.physical_inserts {
            record(*d);
        }
        for (_, _, d) in &log.template_inserts {
            record(*d);
        }
        for (_, _, d) in &log.method_inserts {
            record(*d);
        }
        for (_, _, d) in &log.overload_inserts {
            record(*d);
        }
    }
    submit_at
}

/// Diagnostic (`SURGE_REPLAY_DAG=1`): estimates the conflict dependency DAG's
/// parallel-round ceiling. A conflict's level is `1 + max level of an earlier
/// conflict whose published insert it misses`; the max level is the number of
/// serial rounds an idealized round-based replay would need.
pub(crate) fn report_conflict_dag(logs: &[Option<FileCacheLog>]) {
    if std::env::var_os("SURGE_REPLAY_DAG").is_none() {
        return;
    }
    let predicted = predict_conflicts(logs);
    let mut digest_level: FxHashMap<u64, u32> = FxHashMap::default();
    let mut histogram: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    let mut max_level = 0u32;
    let mut conflicts = 0u32;
    for (index, log) in logs.iter().enumerate() {
        let Some(log) = log else { continue };
        if !predicted[index] {
            continue;
        }
        conflicts += 1;
        let mut level = 1u32;
        for digest in &log.misses {
            if let Some(&producer_level) = digest_level.get(digest) {
                level = level.max(producer_level + 1);
            }
        }
        max_level = max_level.max(level);
        *histogram.entry(level).or_default() += 1;
        let mut publish = |digest: u64| {
            let entry = digest_level.entry(digest).or_insert(level);
            *entry = (*entry).max(level);
        };
        for (_, _, d) in &log.generic_inserts {
            publish(*d);
        }
        for (_, _, d) in &log.instantiation_inserts {
            publish(*d);
        }
        for (_, _, d) in &log.physical_inserts {
            publish(*d);
        }
        for (_, _, d) in &log.template_inserts {
            publish(*d);
        }
        for (_, _, d) in &log.method_inserts {
            publish(*d);
        }
        for (_, _, d) in &log.overload_inserts {
            publish(*d);
        }
    }
    eprintln!(
        "[stc-dag] conflicts={conflicts} max_level(round_ceiling)={max_level} level_hist={histogram:?}"
    );
}

/// Per predicted-conflict position, its conflict-DAG dependency positions (the
/// first-writers of keys it missed, plus its overlay producers, all `< index`).
/// Computed before the commit walk consumes the logs. Non-conflicts get an empty
/// list. Feeds [`report_critical_path`].
pub(crate) fn compute_conflict_deps(logs: &[Option<FileCacheLog>]) -> Vec<Vec<usize>> {
    let predicted = predict_conflicts(logs);
    let n = logs.len();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut first_writer: FxHashMap<u64, usize> = FxHashMap::default();
    for (index, log) in logs.iter().enumerate() {
        let Some(log) = log else { continue };
        if predicted[index] {
            let mut set: FxHashSet<usize> = FxHashSet::default();
            for digest in &log.misses {
                if let Some(&producer) = first_writer.get(digest)
                    && producer < index
                {
                    set.insert(producer);
                }
            }
            for &producer in &log.overlay_deps {
                if producer < index {
                    set.insert(producer);
                }
            }
            deps[index] = set.into_iter().collect();
        }
        let mut record = |digest: u64| {
            first_writer.entry(digest).or_insert(index);
        };
        for (_, _, d) in &log.generic_inserts {
            record(*d);
        }
        for (_, _, d) in &log.instantiation_inserts {
            record(*d);
        }
        for (_, _, d) in &log.physical_inserts {
            record(*d);
        }
        for (_, _, d) in &log.template_inserts {
            record(*d);
        }
        for (_, _, d) in &log.method_inserts {
            record(*d);
        }
        for (_, _, d) in &log.overload_inserts {
            record(*d);
        }
    }
    deps
}

/// Diagnostic (`SURGE_CRITPATH=1`): the out-of-order-commit ceiling. Given each
/// conflict position's measured recompute time (`micros[k]`) and the conflict
/// DAG (`deps`), computes the longest *weighted* dependency chain — the critical
/// path that even a perfect topological, unbounded-parallel commit could not
/// beat — alongside the serial sum. If `critical_path ≈ serial_sum` the
/// conflicts form one long chain and out-of-order commit cannot help; if
/// `critical_path ≪ serial_sum` there is parallelism to exploit.
pub(crate) fn report_critical_path(deps: &[Vec<usize>], micros: &FxHashMap<usize, u128>) {
    if std::env::var_os("SURGE_CRITPATH").is_none() {
        return;
    }
    let n = deps.len();
    let mut finish: Vec<u128> = vec![0; n];
    let mut prev: Vec<Option<usize>> = vec![None; n];
    let mut serial_sum: u128 = 0;
    let mut critical_path: u128 = 0;
    let mut best_end: Option<usize> = None;
    let mut conflicts = 0u32;
    // Positions are already in ascending serial order and every dependency is
    // `< index`, so a single forward pass is a valid topological DP.
    for index in 0..n {
        let weight = *micros.get(&index).unwrap_or(&0);
        if deps[index].is_empty() && weight == 0 {
            continue;
        }
        conflicts += 1;
        serial_sum += weight;
        let mut dep_finish: u128 = 0;
        let mut dep_prev: Option<usize> = None;
        for &producer in &deps[index] {
            if finish[producer] > dep_finish {
                dep_finish = finish[producer];
                dep_prev = Some(producer);
            }
        }
        finish[index] = dep_finish + weight;
        prev[index] = dep_prev;
        if finish[index] > critical_path {
            critical_path = finish[index];
            best_end = Some(index);
        }
    }
    let mut chain_len = 0u32;
    let mut cursor = best_end;
    while let Some(k) = cursor {
        chain_len += 1;
        cursor = prev[k];
    }
    let ideal_8 = (serial_sum / 8).max(critical_path);
    eprintln!(
        "[stc-critpath] conflicts={conflicts} serial_sum={:.0}ms critical_path={:.0}ms \
         ideal_parallel(8core)={:.0}ms critical_chain_len={chain_len} speedup_ceiling(inf)={:.2}x \
         speedup_ceiling(8core)={:.2}x",
        serial_sum as f64 / 1000.0,
        critical_path as f64 / 1000.0,
        ideal_8 as f64 / 1000.0,
        serial_sum as f64 / critical_path.max(1) as f64,
        serial_sum as f64 / ideal_8.max(1) as f64,
    );
}

/// Validates one file's log against everything published so far. On a clean
/// verdict the file's insertions are published into the live maps (in the
/// file's own insertion order) and their digests join `published`.
///
/// Validation is by digest presence (equality-consistent structural digests).
/// A value-based refinement — commit clean when a colliding miss's value equals
/// the published value — was investigated and rejected as unsound: a worker
/// computing against the incomplete fan-out snapshot over-recurses on keys
/// serial would hit and interns spurious sub-instantiations, which pollute the
/// committed cache for later files even when the worker's own diagnostics match
/// serial (and are invisible to a value check, being new keys that never
/// collide). Only a computation reading the *complete* committed state at its
/// position avoids over-recursion — the inline recheck or a validated replay
/// (`crate::replay`). See `docs/perf/TRPC-ORDERED-DELTA-REPLAY.md`.
pub(crate) fn commit_file_log(
    live: &LiveCacheHandles,
    log: &FileCacheLog,
    published: &mut FxHashSet<u64>,
    dirty_files: &FxHashSet<usize>,
    cap: usize,
    stats: &mut StcCommitStats,
) -> CommitVerdict {
    stats.files += 1;
    if !log.misses.is_disjoint(published) {
        stats.miss_conflicts += 1;
        return CommitVerdict::MissConflict;
    }
    if log
        .overlay_deps
        .iter()
        .any(|file| dirty_files.contains(file))
    {
        stats.dependency_conflicts += 1;
        return CommitVerdict::DependencyConflict;
    }
    apply_file_log(live, log, published, cap, stats);
    stats.clean_commits += 1;
    CommitVerdict::Clean
}

/// Validates a single-file *replay* log (produced by a pool thread reading the
/// live committed store) against the published-digest set, and publishes its
/// insertions if valid. Returns whether it was applied.
///
/// A replay reads only the committed store and its own private overlay, so its
/// log never carries overlay dependencies — validation is purely
/// `misses ∩ published == ∅`. Under strict in-order publication a valid replay
/// matches the serial run at its position exactly (see `crate::replay`): a miss
/// disjoint from `published` proves the replay did not over-recurse on any key
/// serial published before this position, so its cache insertions match serial.
/// Unlike an inline recheck, a cap-blocked or already-present insertion is not
/// an error: a position between the replay's read and the frontier may have
/// grown a bucket to the cap, which serial would see identically here.
pub(crate) fn commit_replay_log(
    live: &LiveCacheHandles,
    log: &FileCacheLog,
    published: &mut FxHashSet<u64>,
    cap: usize,
    stats: &mut StcCommitStats,
) -> bool {
    if !log.misses.is_disjoint(published) {
        return false;
    }
    apply_file_log(live, log, published, cap, stats);
    true
}

/// Publishes a validated (or rechecked) file's insertions into the live maps
/// with the same dedup/cap guards the serial intern paths use. Returns whether
/// every insertion was published; a partial publication (cap-blocked or
/// already-present entry) means later files that consumed this file's overlay
/// entries can no longer be validated by digest alone.
pub(crate) fn apply_file_log(
    live: &LiveCacheHandles,
    log: &FileCacheLog,
    published: &mut FxHashSet<u64>,
    cap: usize,
    stats: &mut StcCommitStats,
) -> bool {
    let mut complete = true;

    if !log.generic_inserts.is_empty()
        && let Ok(mut cache) = live.generic.lock()
    {
        for (key, entry, digest) in &log.generic_inserts {
            let bucket = cache.entry(key.clone()).or_default();
            if bucket
                .iter()
                .any(|existing| existing.matches(entry.arguments_digest, &entry.arguments))
            {
                stats.merge_skipped_existing += 1;
                complete = false;
                continue;
            }
            if bucket.len() >= cap {
                stats.merge_cap_blocked += 1;
                complete = false;
                continue;
            }
            bucket.push(entry.clone());
            published.insert(*digest);
            stats.published_entries += 1;
        }
    }
    if !log.instantiation_inserts.is_empty()
        && let Ok(mut cache) = live.instantiations.lock()
    {
        for (key, entry, digest) in &log.instantiation_inserts {
            let bucket = cache.entry(key.clone()).or_default();
            if bucket
                .iter()
                .any(|existing| existing.matches(entry.arguments_digest, &entry.arguments))
            {
                stats.merge_skipped_existing += 1;
                complete = false;
                continue;
            }
            if bucket.len() >= cap {
                stats.merge_cap_blocked += 1;
                complete = false;
                continue;
            }
            bucket.push(entry.clone());
            published.insert(*digest);
            stats.published_entries += 1;
        }
    }
    if !log.physical_inserts.is_empty()
        && let Ok(mut cache) = live.physical.lock()
    {
        for (key, resolved, digest) in &log.physical_inserts {
            if cache.contains_key(key) {
                stats.merge_skipped_existing += 1;
                complete = false;
                continue;
            }
            cache.insert(key.clone(), resolved.clone());
            published.insert(*digest);
            stats.published_entries += 1;
        }
    }
    if !log.template_inserts.is_empty()
        && let Ok(mut cache) = live.templates.lock()
    {
        for (key, template, digest) in &log.template_inserts {
            if cache.contains_key(key) {
                stats.merge_skipped_existing += 1;
                complete = false;
                continue;
            }
            cache.insert(key.clone(), template.clone());
            published.insert(*digest);
            stats.published_entries += 1;
        }
    }
    if !log.method_inserts.is_empty()
        && let Ok(mut cache) = live.methods.lock()
    {
        for (key, function, digest) in &log.method_inserts {
            if cache.contains_key(key) {
                stats.merge_skipped_existing += 1;
                complete = false;
                continue;
            }
            cache.insert(key.clone(), function.clone());
            published.insert(*digest);
            stats.published_entries += 1;
        }
    }
    if !log.overload_inserts.is_empty()
        && let Ok(mut cache) = live.overloads.lock()
    {
        for (key, function, digest) in &log.overload_inserts {
            if cache.contains_key(key) {
                stats.merge_skipped_existing += 1;
                complete = false;
                continue;
            }
            cache.insert(key.clone(), function.clone());
            published.insert(*digest);
            stats.published_entries += 1;
        }
    }

    complete
}
