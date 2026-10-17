# Performance investigation reports

**Every file in this directory is a point-in-time record.** Each documents one
investigation as it stood on its own date: the measurements taken, the changes
landed, and — often more valuable — the designs that were measured and
**rejected**. None of them describes the current state, and their counts
(diagnostic totals, preset totals, wall-clock medians) are stale by
construction.

- Current state: [CURRENT_STATUS.md](../../CURRENT_STATUS.md)
- Benchmark methodology: [BENCHMARKS.md](../../BENCHMARKS.md)
- The rules these investigations must obey: [PERFORMANCE_INVARIANTS.md](../PERFORMANCE_INVARIANTS.md)
- Retained-memory model: [MEMORY_REGIONS.md](../../crates/surge-ts-checker/MEMORY_REGIONS.md)

The reports kept here are the ones source comments or the memory rules cite.
The rest were removed on 2026-10-17; their rejected designs are digested below
so the ideas are not re-proposed, and `git show e5fc8b09:docs/perf/<name>.md`
restores any of them in full.

## Design references (not status reports)

| Document | Subject |
| --- | --- |
| [SPECULATIVE-TRANSACTIONAL-CHECKING.md](SPECULATIVE-TRANSACTIONAL-CHECKING.md) | Design reference for parallelizing semantic analysis |
| [MEMBER-LAZY-EXPANSION.md](MEMBER-LAZY-EXPANSION.md) | Member-level lazy interface expansion (`SURGE_LAZY_IFACE_MEMBERS`, Stage 1 opt-in) |

## Investigation reports, newest first

| Document | Date | Subject |
| --- | --- | --- |
| [LOADER-PARSE-HANDOFF.md](LOADER-PARSE-HANDOFF.md) | 2026-09-11 | Handing the module-graph scan's parses to the checker instead of re-parsing; the per-file text sweeps around it |
| [TANSTACK-QUERY-PROFILE-2026-09-10.md](TANSTACK-QUERY-PROFILE-2026-09-10.md) | 2026-09-10 | Where tanstack-query's time goes; deferred references in named-type resolution |
| [NAMESPACE-INTERFACE-MERGE.md](NAMESPACE-INTERFACE-MERGE.md) | 2026-08-20 | Reopened-namespace interface merging. **Note:** the merge described there as gated off is now **on by default** (opt-out `SURGE_NS_IFACE_MERGE=0`). |
| [MEMORY-OPTIMIZATION-REPORT.md](MEMORY-OPTIMIZATION-REPORT.md) | 2026-07-17 | tRPC peak/finish memory: census evidence, landed changes, rejected designs |
| [TRPC-ORDERED-DELTA-REPLAY.md](TRPC-ORDERED-DELTA-REPLAY.md) | — | Ordered-delta pipelined replay; sound but no wall-time gain (blocker) |
| [TRPC-DEFERRED-RESOLUTION.md](TRPC-DEFERRED-RESOLUTION.md) | — | Publisher-stamped deferred resolution; blocker |
| [TRPC-STC-REPORT.md](TRPC-STC-REPORT.md) | — | Speculative transactional checking, engineering report |

Dates marked `—` are recorded inside the document rather than in its title.

## Rejected designs from removed reports

| Idea | Why it was rejected | Source |
| --- | --- | --- |
| Fx-hash for `type_dedup_fingerprint` | Diagnostic drift: dedup bucketing changes payload sharing, and assignability is `Arc`-pointer-identity-sensitive on cycle edges | `TRPC-5S-REPORT.md` |
| Per-declaration `is_library_scoped` memo | Unsound — the answer is context-sensitive (467K divergences) | `TRPC-5S-REPORT.md` |
| Interner-hit form normalization | +10 diagnostics; nominal wrapping of non-object expansions is semantically visible | `TRPC-5S-REPORT.md` |
| Conditional-result cache, fingerprint index, `resolve_arc` peel-clone elimination, topo-JIT single pass | +4–5% slower, neutral, ~2%, and 3.2× RSS with FP drift respectively | `TRPC-5S-REPORT.md` |
| Restoring the program-wide resolution caches after every file (`SURGE_CHECK_CACHE_ISOLATION`) | Showed rendered diagnostic text is coupled to shared-cache seeding order; kept as a probe only | `TRPC-5S-REPORT.md` |
| Fresh per-module store forks (`SURGE_ANALYSIS_FRESH_STORE`) | Broke pointer-identity invariants without fixing the zod message flip | `TRPC-5S-FINAL.md` |
| Parallel analysis/check under `--jobs auto` | +0.6 GB for ≤ 0.3 s at the commit-tail costs of the time | `TRPC-5S-FINAL.md` |
| `SURGE_TIMINGS` for parallel measurement | Per-file mutex traffic inverts parallel phase timings; use `SURGE_STAGE_TIMES` + `SURGE_STC_STATS` | `TRPC-5S-FINAL.md` |
| Reserving on real miss sets plus a topological wave commit | Unnecessary: worker logs already predict publishes; the stale replays came from a too-coarse validation digest | `TRPC-FINE-CONFLICT-DIGEST.md` |
| Program-wide caching of sentinel-embedding expansions | Changed real zod diagnostics (four `TS2339` vanished, one message drifted) — degraded results are uncacheable | `SIGNATURE-CONTEXT-GENERIC-CACHE.md` |
| Authoritative-map substitution when recovering stale declaration environments | Exposed a two-copy nominal-identity gap in dependency `.d.ts` resolution (`Task` vs `Task`, `QueryClient` vs `QueryClient`); reverted | `CLEAN-GENERIC-BASE-EXPANSION.md` |
