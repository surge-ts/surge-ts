# Program Checking

`surge-ts-checker` checks a program — a set of loaded files checked together,
with module bindings, ambient declarations and the bundled standard library
(see [STANDARD_LIBS.md](STANDARD_LIBS.md)) applied across them. What the checker
supports right now is recorded in [CURRENT_STATUS.md](../../CURRENT_STATUS.md);
this file describes how a run is organized.

## Public API

- `check_source(source_text, file_name)` /
  `check_source_with_options(source_text, file_name, options)` — one file, no
  sibling files read.
- `check_program(files)` / `check_program_with_options(files, options)` — a
  program of `SourceFileInput`s.

TS5112 and the other config-level diagnostics are emitted by the CLI before
semantic checking; these APIs never produce them.

`CheckerOptions` is shared as a single `Arc<CheckerOptions>` across every
context in a run (the options carry the project-wide module-resolution
tables); internal shadow contexts go through
`CheckerContext::new_with_shared_options` rather than deep-cloning the
options.

## Checking Execution Model

The orchestrator lives in `src/program/mod.rs`. Each
`check_program_with_stats_and_jobs` run creates a fresh canonical
`ProgramTypeStore` and installs it thread-locally for the run (workers install
the same store on their threads); all program caches and the store are torn
down at end of run (`clear_program_type_caches` + `ProgramTypeStore::clear`).

### Serial vs parallel

- `jobs = 1` checks files serially; `jobs = 0` (the CLI's auto mode) selects a
  worker count from available cores capped by parsed statement count
  (`MIN_STATEMENTS_PER_WORKER`), so tiny programs stay serial; any other value
  requests that many workers (capped by file count).
- Parsing may also fan out to parse workers; loading, graph construction,
  declaration collection, and module binding remain serial.
- Parallel workers pull file indices from a shared atomic counter; results are
  merged by loaded-file order (`file_index`), never completion order, so
  diagnostics are deterministic across worker counts. Serial/parallel
  diagnostic equality is an asserted invariant.

### Checker-context reuse and per-file reset

Both modes reuse one `CheckerContext` per worker — serial checking uses a
single reused context for the whole pass (cloning the large context per file
was a measured ~3% of check-phase time on tRPC). `check_program_file` starts
with `CheckerContext::begin_file_check`, the file-region reset:

- `resolved_named_types` is replaced with a fresh map (swapped, not cleared in
  place — resolutions depend on the consumer file's environment, and retained
  declaration environments may still reference the previous file's map);
- the diagnostic dedup index is cleared (diagnostics themselves were taken at
  the previous file's end);
- the `push_utility_diagnostic_once` keys use a baseline/overlay split: keys
  inherited from before the check phase form a shared baseline, and only the
  per-file overlay is cleared, so a reused context behaves byte-identically to
  a fresh clone without cloning the key set per file.

Any new per-file transient state must join this reset or it will leak across
files on a reused worker.

### Preliminary vs final analysis passes

Module analysis runs twice (`collect_module_analyses_with_bindings`): a
preliminary round makes types and import/export shapes available so the
export-table/import-binding/resolution-scope fixpoint can run, and a final
round re-analyzes every module against the settled bindings. Two rules follow
from the rounds being asymmetric:

- **Preliminary results must not install first-wins global state.**
  `declare global` augmentation *values* are lowered only in the final round
  (the `lower_global_augmentation_values` flag): insertion into
  `ambient_global_symbols` is first-wins, so a value typed against the
  incomplete preliminary environment would permanently shadow the correctly
  typed final-round value.
- Preliminary analyses, bindings, scopes, and per-round export tables are
  dropped at the `preliminary_release` boundary once the final round
  supersedes them (see MEMORY_REGIONS.md).

### Lexical declaration environments for dependency files

A declaration's body must resolve in its declaring module's scope, not the
consumer's:

- `module_scope_by_file` maps each module's source file to its resolution
  scope (local declarations + resolved imports). It is installed before the
  *final* analysis round — signature collection resolves parameter types
  through local aliases whose pre-attached scope carries no import layers —
  and refreshed before the check phase. The preliminary round deliberately
  runs without it (its outputs are superseded, and resolving the full import
  graph twice measurably regresses time/memory on large cyclic programs).
- `module_local_values_by_file` is the value analogue: `typeof <localValue>`
  inside an imported alias body resolves against the declaring module's
  values. It is built once before the (possibly parallel) check phase so all
  jobs share it read-only and results stay order-independent.
- While a dependency `.d.ts` body is expanded from another file, name lookup
  ignores the consuming module's local declaration table
  (`lookup_ignores_local_table` in `src/context/mod.rs`) so a consumer-local name
  cannot shadow the dependency's own lexical scope.

## Shared symbol tables

- Value symbol tables store shared `SymbolInfo` handles, and the table map is
  copy-on-write (`Arc<HashMap<..>>` with `Arc::make_mut`), so the multi-pass
  module-binding fixpoint's clones share one map until one is mutated.
- `SymbolInfo` entries are immutable once shared. Narrowing or assignment-style
  updates create a replacement `SymbolInfo` and swap the table handle instead of
  mutating a shared payload in place.
- Per-run memos (path canonicalization, relative module resolution) are
  thread-local and cleared at the start of each `check_program` run, so they
  never leak resolved indices or canonical paths across runs.
- Declaration diagnostics keep the declaration's file name; diagnostics on a
  consuming expression keep the consumer's.

## Ambient Globals

Ambient globals from loaded `.d.ts` files are gathered into `ambient_global_symbols` and `ambient_global_type_declarations`, then mixed into modules and scripts.

## Diagnostic Profiles

The checker supports two diagnostic profiles, controlled via `CheckerOptions.diagnostic_profile` or the `--diagnosticProfile` CLI flag:

1. **`Tsc` (Default)**: Aims to match the TypeScript compiler (`tsc`) exactly. This includes applying targeted type widening (like on `satisfies`) and allowing known diagnostic cascades (like outer assignability failures) to appear if TypeScript emits them.
2. **`Native`**: Uses `surge-ts`-specific behaviors. For example, it aggressively returns `Unknown` from failed contextual checks (like `satisfies` failures) to suppress noisy downstream cascade errors. This produces a cleaner developer experience but diverges from the TypeScript compiler baseline.

The `compat-projects` oracle testing runs exclusively in `tsc` profile.
`CheckerOptions::default().diagnostic_profile` is `DiagnosticProfile::Tsc`, and the CLI default stays `tsc` unless `--diagnosticProfile native` is explicitly requested.
