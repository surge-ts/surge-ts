# surge-ts tRPC Memory Optimization — Engineering Report

> **Historical engineering report — a point-in-time record (2026-07-17).** The
> counts it quotes (oracle sweep 83/83, workspace tests 1521, per-project
> diagnostic totals) were current at commit `6fc9e6c` and have since changed.
> The *mechanisms* and the rejected designs it records are still load-bearing —
> see [crates/surge-ts-checker/MEMORY_REGIONS.md](../../crates/surge-ts-checker/MEMORY_REGIONS.md)
> and [PERFORMANCE_INVARIANTS.md](../PERFORMANCE_INVARIANTS.md). For the current
> state see [CURRENT_STATUS.md](../../CURRENT_STATUS.md).

Baseline commit: `6fc9e6c`. Landed as the six-commit series ending at
`8f0c3a9` (`feat(metrics): add retained-memory census and stage-pause
instrumentation`). All figures: `.local-projects/trpc`, system allocator,
macOS `phys_footprint` (`/usr/bin/time -l` "peak memory footprint", and the
`SURGE_RSS=1` per-stage `fp`/`fp_peak` columns).

Two result sets are reported: **original** (the 2026-07-17 optimization
session) and **reproduced** (rebuilt from the recovered, overload-free source
that actually landed; only these count as final).

## Results

| Metric | Baseline (6fc9e6c) | Original session | Reproduced from landed source |
|---|---|---|---|
| Peak physical footprint (ledger, `time -l`) | 4.02–4.21 GB | 1.89–2.01 GB | **1.88–1.90 GB** |
| Peak physical footprint (stage `fp_peak`) | 3.78–3.81 GB | 1.76–1.78 GB | **1.75 GB** |
| Finish physical footprint | 1.94–1.97 GB | 0.55–0.59 GB | **0.56 GB** |
| Wall, jobs=1 | 19.2–22.0 s | 17.8–19.1 s | **19.1–20.1 s** |
| tRPC diagnostics (2190) | `a84083…` | byte-identical | **byte-identical, jobs=1 and jobs=auto** |
| zod / ky / ofetch (913 / 0 / 6) | — | byte-identical | **byte-identical, jobs=1 and jobs=auto** |
| Oracle sweep | 83/83 | 83/83, 0 gating drift | **83/83, 0 gating drift** (3 pre-existing non-gating message drifts) |
| Workspace tests | 1521 | 1520 + 1 updated | **1521/1521 pass** |
| Live fallback FunctionType payloads at peak | ~278k | ~102–107k | **103,358** (gate ≤200k) |

Full diagnostic hashes (SHA-256 of the diagnostic output, identical for
baseline and optimized, serial and jobs=auto):

- tRPC `a84083779c33094c8bca1cebb959c10ab43118833408254bc8b14b686afb793d`
- zod `e52bb564ab3adb52b207029906b97376284466585023e54eb3023ee8c8c37c16`
- ky `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` (empty)
- ofetch `788fe1ac9d80e32661fd37b45f852e984a4995ef50036ceaf3a0c27cb07f7ea6`

## Why the peak was 3.8 GB (census + vmmap + malloc_history evidence)

At the pre-check boundary, vmmap showed **3.6 GB genuinely live in 17.5 M
allocations (~206 B avg), 4% fragmentation** — live data, not allocator slack.
The `SURGE_RETENTION_CENSUS=1` walk attributed it:

- **Declaration environments: 1.03 GB.** 49,931 interned
  `DeclarationEnvironmentData` captures, each owning a `TypeDeclarationTable`
  index copy (~214 MB of indexes), a captured working value-symbol table, and —
  via COW-defeated `record_declaration_span` — **5.9 M declaration-span entries
  (283 MB)**: every environment interned between two span recordings froze a
  32 KB snapshot of the ambient span map.
- **Canonical type store: 541 MB unreachable from any live owner.** The store
  held strong `Arc`s forever, so every transient pass's interned types
  accumulated for program lifetime.
- **Module-analysis products: ~470 MB** (export tables 257 MB, local symbols
  99 MB, decl tables + parsed bodies ~155 MB inside those groups).
- **Import bindings: 117 MB** — per-importer deep copies of qualified
  namespace type exports, retained across 3 binding generations.
- Instantiation caches 63 MB, lazy-resolver captures 315k/126 MB,
  ASTs ~400 MB, plus Arc headers/map slack.

## Why the finish was 1.96 GB

Two causes, both fixed:

1. **A real leak**: `CheckerArena::alloc_type_declaration_payload` stored
   payloads via `MaybeUninit` — `Drop` never ran, so every declaration
   payload's `String`s and `Arc<InterfaceBody>`/`Arc<TypeAliasBody>`/scope
   refcounts leaked to process exit (~400 MB ownerless live at finish,
   proven by `malloc_history` on a paused process).
2. **Fragmentation**: at finish only 552–773 MB was live; 61–66% of dirty+swap
   was freed-small-block fragmentation, plus dead structures (shared_state,
   parse trees, TLS caches) that were dropped only after the measurement point.

## Changes landed (commit by commit)

1. `f518a81` **feat(types): use weak payload retention in canonical type
   stores** — bucket entries hold `Weak` payloads (functions, parameter lists,
   unions, property maps); dead entries swept on next bucket scan; IDs
   monotonic, never reused, so identity fast-paths cannot ABA. Payloads live
   exactly as long as consumers. Largest single win (~−1.3 GB peak).
2. `1ef6d85` **fix(arena): run destructors for arena-owned payloads** — a
   `pending_drops` list runs each payload's typed `drop_in_place` when the
   last arena handle drops. Fixes the finish-footprint leak.
3. `a61341b` **perf(checker): share qualified-import payloads with
   owning-arena retention** — `TypeDeclarationTable::insert_shared_from`
   adopts the exporter's payload pointer and retains its owning arena per
   payload (`foreign_payload_arenas`), replacing per-importer clones in
   `copy_qualified_type_exports`; `get_handle` hands back the true owning
   arena. Adds the `(instance_id, version)` mutation stamp and `Arc<str>`
   interface-fragment file names.
4. `3181560` **fix(checker): compact declaration-environment captures** —
   environments capture an `Arc<TypeDeclarationTable>` snapshot deduplicated
   by the mutation stamp (one snapshot per burst), span-free symbol tables
   (`clone_for_environment_capture`), and an empty working value table
   (typeof falls through ambient → `module_value_fallback` →
   `module_local_values_by_file`; corpus- and sweep-verified byte-identical).
5. `e5b7ac6` **perf(checker): release superseded analysis, AST, and cache
   state early** — declaration-file AST bodies filtered to import/export
   statements after final module analysis; superseded binding/scope
   generations dropped before each rebuild; per-file AST +
   `module_analyses[i]` + `module_import_bindings[i]` freed progressively in
   the serial check loop; `shared_state`/`parsed_files` dropped before the
   finish measurement; run-scoped TLS caches cleared at teardown;
   `malloc_zone_pressure_relief` at generation boundaries and every 256 files.
6. `8f0c3a9` **feat(metrics): add retained-memory census and stage-pause
   instrumentation** — `SURGE_RETENTION_CENSUS=1` full retained-heap census
   with per-owner-group attribution and fallback classification;
   `SURGE_PAUSE_AT_STAGE=<label>` self-SIGSTOP for `vmmap`/`malloc_history`
   attachment. Opt-in and diagnostics-neutral.

## Rejected designs (measured, reverted)

- **Re-export (`export *`) payload sharing** — zod 913→914 + message-render
  drift: collapsing per-table clones into one shared payload changes which
  first-wins expansion later consumers observe.
- **Pruning `program_instantiations` entries with `strong_count == 1`** at the
  pre-check boundary — same zod drift class (and memory-neutral on tRPC).
- Rule derived: *any change to expansion-cache lifetime or identity shifts
  first-wins expansion results on zod.* The weak store is safe precisely
  because an entry dies only when no consumer exists.

## Fallback FunctionType classification

Of live fallback payloads at peak: **99.3% Unknown-containing** (the
degradation sentinel — canonicalizing them program-wide is forbidden),
~0.4% over-budget fingerprints, ~0.5% internable-but-fell-back, 0
context-retaining references. Their retention was cut 278k→~103k live by
fixing their *owners'* lifetimes, which is the sound lever for degraded
values.

## Remaining live owners at the ~1.8 GB peak (quantified)

- Module-analysis export tables 257 MB + local symbols 98 MB — eager
  value/signature type graphs. Next lever: lazy export-table value symbols
  (defer to first import-site use). Architectural.
- Parsed declaration bodies ~155 MB — `ParsedType` is `String`/`Vec`-heavy;
  interning `ParsedNamedType.name`/member names is the lever. Invasive to the
  parser surface.
- Environments 80 MB, instantiation caches 64–77 MB (CPU-load-bearing),
  resolver captures ~50 MB, root ASTs ~150 MB until checked.
- ~0.5 GB of Arc control blocks, size-class rounding, map capacity, and
  fragmentation — a region-allocation project.

The gap to a 1.5 GiB peak (~0.3 GB) needs the first or last of these; both
are multi-day representation projects, and the two cheap cache-lifetime
shortcuts were tried and measurably drift diagnostics.

## Recovery note

The optimization was recovered from a working tree where it was mixed with
in-progress function-overload work. The two were separated hunk-by-hunk;
the overload WIP lives on `recovery/overload-only` (carries the plural
`call_signatures` AST surface, TS2769, ordered overload groups on
`FunctionType`, and three oracle fixtures). One shared hunk existed: the
census's iteration over interface call signatures, adapted to the singular
baseline field on this branch. The combined pre-separation tree is preserved
at `backup/overload-memory-worktree`.
