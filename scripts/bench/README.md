# Compiler Speed Benchmark Harness

This directory contains the developer-facing regression benchmark (`compare-compilers.ts`) for measuring wall-clock performance and diagnostic drift.

It is **not** a marketing benchmark. It compares the same `tsconfig.json` inputs across:

1. JavaScript TypeScript compiler (`tsc` = TypeScript 6.x, the `typescript-6` workspace alias, kept as the slow speed/memory reference)
2. TypeScript native compiler (`tsgo` = TypeScript 7.0, the canonical `typescript` package, and the diagnostic baseline)
3. `surge-ts` CLI (using the release binary, not `cargo run`)

## Requirements

1. **Release Binary Required:** The benchmark tests the prebuilt release binary, not debug builds. Build it first:

    ```bash
    cargo build --release -p surge-ts-cli
    ```

2. **`tsgo` is Included When Available:** the native TypeScript 7.0 compiler ships as the workspace `typescript` package, so no global install is needed. If it is resolvable, the harness benchmarks it automatically and still cleanly skips it when it is unavailable.
3. **TS 7-Oriented Fixtures:** Committed fixtures under `tests/compat-projects/` must not use `ignoreDeprecations` in their `compilerOptions`. If a fixture triggers a TS 6 deprecation warning, fix the fixture config instead of suppressing the diagnostic. The harness actively guards against `ignoreDeprecations` usage.

## What It Measures

The benchmark compares **no-emit project checking** only:

- `tsc --noEmit --pretty false --project <tsconfig>`
- `tsgo --noEmit --pretty false --project <tsconfig>`
- `surge-ts --project <tsconfig> --format json --maxDiagnostics 10000`
- `surge-ts --project <tsconfig> --format json --maxDiagnostics 10000 --jobs <auto|n>` for deterministic project-checking measurements; `auto` (the default) sizes workers by cores and workload, or pass an explicit count

It does not measure watch mode, incremental builds, project references, emitting, or editor performance.

**Diagnostic Drift:** Speed is meaningless if the diagnostic surface is wrong. The script runs a single check per tool to capture diagnostics and reports drift against `tsgo` (TypeScript 7.0), the same reference the oracle compares against, so a tool that matches TS 7 is never reported as drifting because TS 6 disagreed. Labels name the baseline they were measured against (`exact vs tsgo`, `known delta vs tsgo`, `parse failed`). When the `typescript` package is not resolvable, `tsc` (TS 6) becomes the baseline and the labels say so.

Speed and memory ratios are separate: they stay relative to `tsc` (TS 6), which is kept as the slow reference point.

## Usage

Run a specific project:

```bash
pnpm run bench:compilers -- --project tests/compat-projects/optional-chaining-basic/tsconfig.json
```

Run a preset:

```bash
pnpm run bench:compilers -- --preset current
```

Run a real project checkout (directory inputs resolve to their `tsconfig.json`):

```bash
pnpm run bench:trpc          # shorthand for the command below
pnpm run bench:compilers -- --project .local-projects/trpc --json .bench/compilers/trpc.json --chart .bench/compilers/trpc.svg --html .bench/compilers/trpc.html
```

`bench:ky`, `bench:trpc`, `bench:zod`, `bench:ofetch`, `bench:tanstack-query`,
`bench:ts-pattern`, `bench:drizzle-orm`, `bench:zustand`, and `bench:unnamed`
are predefined for
the usual local checkouts and write their JSON/SVG/HTML reports to
`.bench/compilers/<name>.*`. Extra flags append after `--`, e.g.
`pnpm run bench:trpc -- --iterations 3 --rustJobs 1`.

Change iterations, generate visual reports, and output JSON:

```bash
pnpm run bench:compilers -- --preset current --iterations 10 --warmup 2 --json .bench/compiler-bench.json --chart .bench/compiler-bench.svg --html .bench/compiler-bench.html
```

Measure Rust at a specific worker count (default is `auto`; pass `1` for serial or an explicit count to calibrate):

```bash
pnpm run bench:compilers -- --project tests/compat-projects/parallel-ordering-basic/tsconfig.json --rustJobs 4
```

Generate visual reports from an existing JSON run without re-running compilers:

```bash
pnpm run bench:compilers -- --fromJson .bench/compiler-bench.json --chart .bench/compiler-bench.svg --html .bench/compiler-bench.html
```

`tsgo` is enabled by default when available:

```bash
pnpm run bench:compilers -- --preset current
```

If you want to make the intent explicit, `--include-tsgo` remains accepted as a no-op flag.

Generate and run a synthetic scale fixture:

```bash
pnpm run bench:compilers -- --generate scale-small --files 50 --symbols 200
```

*(Generated fixtures are written to `.bench/generated/` and not committed).*

## Archiving Runs

`archive-run.ts` wraps the existing benchmark and real-project commands and saves
their raw output plus a small summary into a timestamped local directory under
`.bench/runs/<timestamp>/`. It does not change what those commands measure; it
only captures and labels their output so before/after runs are easy to compare.

```bash
pnpm run bench:archive                                  # compiler benchmark only (default)
pnpm run bench:archive -- --bench                       # explicit compiler benchmark
pnpm run bench:archive -- --label builtin-removal-before
pnpm run bench:archive -- --out .bench/runs/custom-name # override the output directory
pnpm run bench:archive -- --dryRun                      # print commands + paths, run nothing
```

Each run directory contains the captured `bench-compilers.txt` log (and the bench
`--json`), plus `summary.json` and `summary.md` with the git branch/commit,
per-command pass/fail and exit codes, and parsed benchmark medians. A partial
summary is still written when one command fails; the process exits non-zero if any
underlying command fails. `.bench` is git-ignored, so archived runs are local by
default.

## Running Benchmark Tests

To verify the harness itself:

```bash
cargo build --release -p surge-ts-cli
pnpm run bench:test
```

## Report Outputs

Every benchmark run also samples **peak memory** per iteration through the
same `/usr/bin/time` wrapper as `measure-project.ts` (macOS `-l`, Linux `-v`),
so wall time and memory come from the same invocations. On macOS the metric is
**phys_footprint** (`time -l`'s "peak memory footprint" line — the
Activity-Monitor-comparable number); elsewhere it falls back to maximum RSS.
The `source` field records which metric was used. Where peak memory cannot be
measured, timing still works and the memory fields are null.

- `--json` writes `{ meta, results }`: `meta` records the timestamp, git
  branch/commit, CPU model and core count, platform, Node version, the exact
  `tsc` (TS 6) and `tsgo` (TS 7) package versions, and iteration/warmup
  counts; `results` holds per-project median/min/max/runs, drift, and
  median/min/max peak memory per tool. Legacy bare-array JSON files are
  still accepted by `--fromJson`, `bench:archive`, and `measure-project.ts`.
- Reports label the tools with their TypeScript major version — `tsc (TS 6)`
  is the legacy JS speed reference, `tsgo (TS 7)` the native compiler and the
  diagnostic baseline — so the slow reference is never mistaken for current
  TypeScript.
- `--chart` renders a single SVG with two stacked panels: wall time (median
  bars on a time axis, min–max whiskers, speed multipliers vs the tsc
  reference, colored diagnostic-drift badges) and peak memory (median bars on
  a MB/GB axis, whiskers, memory ratio vs tsc). The memory panel is omitted
  when no memory was sampled. Run metadata is in the header.
- `--html` renders a tabbed page — **Wall time** and **Peak memory** tabs —
  each with its chart panel and a per-project stats table
  (median/min/max/spread/runs/vs-tsc/drift, and memory/of-tsc/source on the
  memory tab), plus a run-metadata panel. Without memory data the page is a
  single untabbed timing view.

## Interpreting Results

- Benchmark results are local-machine-relative. SVG/HTML reports are visualization aids, not cross-machine marketing claims.
- JSON, SVG, and HTML labels include the Rust job count when one is recorded so mixed serial/parallel runs are not ambiguous.
- Compare wall-clock timings alongside the diagnostic drift status.
- This is not yet a claim of full compiler parity. See `REAL_PROJECT_COMPAT.md` for current compatibility limitations.

## Diagnostic Accuracy vs bolt-ts (`compare-checkers.ts`)

Scores `surge-ts` and [bolt-ts](https://github.com/bvanjoi/bolt-ts) against
`tsgo` (TypeScript 7.0) as true/false positives and negatives, using the same
`file:line:code` multiset key the oracle sweep gates on. It measures accuracy,
not speed; the per-tool time column is a single run.

Setup — bolt-ts is expected beside this checkout (or point `BOLT_TS_BIN` at it):

```bash
git clone https://github.com/bvanjoi/bolt-ts ../bolt-ts
(cd ../bolt-ts && cargo build --release -p bolt_ts_compiler)
cargo build --release -p surge-ts-cli
```

The default tier is **upstream held-out**: TypeScript's own test cases minus
every case either checker uses in its own test suite (bolt-ts's
`tests/compiler`, surge-ts's `tests/upstream` manifest), matched by name,
option-suffixed name, and whitespace-insensitive content. It is the only tier
neither checker was tuned on. Provision the cases at the commits typescript-go
pins (TypeScript submodule `c3bd12d8`, typescript-go `2c251880`):

```bash
git clone --filter=blob:none --no-checkout https://github.com/microsoft/TypeScript.git .local-projects/typescript-cases
(cd .local-projects/typescript-cases && git sparse-checkout set --no-cone 'tests/cases/compiler/**' 'tests/cases/conformance/**' && git checkout c3bd12d888b86f676718b16e64d7d2abcb423514)
git clone --filter=blob:none --no-checkout https://github.com/microsoft/typescript-go.git .local-projects/typescript-go-cases
(cd .local-projects/typescript-go-cases && git sparse-checkout set --no-cone 'testdata/tests/cases/**' && git checkout 2c251880acd98bad0fda5e01985b10eb7821d06a)
```

Each case becomes a project the way TypeScript's harness reads it (`// @option`
directives, `@filename` units, the first value of an option variation); emit-only
options are dropped since every tool runs `noEmit`, and a case tsgo rejects with
TS5xxx is `invalid-config`, not scored. Cases needing harness machinery
(`@link`, `@currentDirectory`, a case-supplied `tsconfig.json`) are skipped.

Run:

```bash
pnpm run bench:checkers                          # upstream held-out, hash-sampled 1000 (default)
pnpm run bench:checkers -- --upstreamLimit 0     # every held-out case
pnpm run bench:checkers -- --fixtures            # tests/compat-projects (surge-biased)
pnpm run bench:checkers -- --corpora             # provisioned .local-projects corpora, one at a time (surge-biased)
pnpm run bench:checkers -- --all --filter react  # substring filter over target names
pnpm run bench:checkers -- --upstreamLimit 0 --strict  # every held-out case, strict match key
```

The default match key is the oracle gate key, `file:line:code`. `--strict`
scores on `file:line:column:message:code` instead (the message is tsc's first
line), so a diagnostic surge reports at the right line with a different column
or message text counts as one false positive and one false negative.

Every spawned tool is polled for memory (its whole process group) and killed
past `--maxMemory` (default 2048 MB) or, for corpora, `--corpusMaxMemory`
(default 6144 MB); the target reports `memory limit` and is not scored. macOS
enforces no address-space rlimit, and bolt-ts runs away on some upstream cases
fast enough to take a 16 GB machine down inside the timeout. On macOS the
measure is physical footprint (`proc_pid_rusage`, read by a `python3` sidecar),
not resident size: under memory pressure the kernel compresses and swaps a
runaway's pages out of its resident set, so its RSS stays under the cap while
its footprint grows past physical memory. Elsewhere, or without `python3`, it
is `ps` resident size. `--jobs` is lowered so jobs × `--maxMemory` stays within
half of physical memory.

The tsgo baseline (the `--noEmit` check and `--listFilesOnly`) dominates an
upstream run's wall time and does not depend on either checker, so for upstream
cases it is cached under `.bench/checkers-cache/tsgo/`, keyed on the tsgo
version, the materialized case directory's path, and every file in it. A rerun
with the same `--out` skips tsgo entirely; a different `--out` misses, because
tsgo's output names files by path. Runs that hit `--timeout` or `--maxMemory`
are not cached. Fixtures and corpora are never cached (they read files outside
their directory). Pass `--noBaselineCache` to recompute, or delete the
directory after changing anything else tsgo reads (e.g. workspace `@types`).

Reports land in `.bench/checkers/report.{md,json,svg,html}` (`--out <dir>` to
change). The SVG and HTML reuse the compiler benchmark's panel renderer and page
chrome from `report.ts`. Re-render them from a previous run without re-checking:

```bash
pnpm run bench:checkers -- --fromJson .bench/checkers/report.json
```

Accuracy is reported twice: over the targets both checkers completed (the fair
comparison), and over each checker's own completed targets. Crashes and
timeouts are counted, never scored.

How bolt-ts output is read, and what that costs:

- bolt-ts prints miette reports with message text but no `TSxxxx` code. Codes
  are recovered by matching the text against the error templates in the
  TypeScript 6 JS compiler's diagnostic table. Text it words differently from
  tsc is **unmapped**: excluded from FP, while the tsgo diagnostic it stood for
  still counts as FN. The report lists the most frequent unmapped shapes. A
  short `boltRewordings` table maps one-to-one rewordings (bolt-ts's
  `Property 'x' is missing.` is TS2741); ambiguous wordings stay unmapped.
- Diagnostics in files outside the tsgo program (`tsc --listFilesOnly`) are
  **out-of-program**, not FP. bolt-ts globs `.js` sources without `allowJs`.
- bolt-ts reads a config only when the file is named exactly `tsconfig.json`,
  ignores `extends` and `files`, cannot expand a directory entry in `include`,
  and rejects option spellings tsc accepts (`nodenext`, `DOM`). Each config is
  therefore resolved with TypeScript's own parser and written to a temp
  `tsconfig.json`: the program's root files as an explicit `include`, and every
  effective option (tsgo's defaults included, e.g. `strict`) in bolt-ts's
  spelling. Options bolt-ts has no field for (`paths`, `types`, `skipLibCheck`,
  …) and libs it does not ship are dropped and reported as config translation
  gaps. Per its README it also does not resolve `exports`/`imports` or
  `node_modules/@types`.
- bolt-ts exits 0 even when its include glob matches nothing. Each run is
  paired with an `include: []` run of the same config; if the real run loaded
  no more files than that lib-only run, the target is **no input** and is not
  scored, like a crash.
- The fixture tier was written to drive surge-ts against tsgo, and the corpora
  are the projects surge-ts burned its false positives down on (three through a
  surge-specific `tsconfig.surge.json`), so both are biased toward surge-ts.
  Only the upstream held-out tier is neutral. Do not turn any tier into a
  compatibility claim.

## Allocator Benchmark (`allocator-bench.ts`)

Compares the `surge` binary built with each supported global allocator:
`system` (default), `mimalloc`, `jemalloc`, `snmalloc`. Allocator selection is a
cargo feature of `surge-ts-cli` only (library crates never set a global
allocator); enabling more than one feature is a compile error.

```bash
pnpm run bench:allocators                       # build all 4 variants, run the full matrix
pnpm run bench:allocators -- --allocators system,mimalloc --iterations 5
pnpm run bench:allocators -- --scenario large --skipBuild   # reuse .bench/allocators/bin
```

What it does:

1. Builds each variant (`cargo build --release -p surge-ts-cli [--features <alloc>]`)
   and copies the binary to `.bench/allocators/bin/surge-<alloc>`.
2. Verifies each binary self-reports its allocator (`SURGE_PRINT_ALLOCATOR=1 surge`).
3. Runs a scenario matrix: small single-file cold run, medium project at
   `--jobs 1` and `--jobs auto` (`.local-projects/ky` when present, otherwise a
   generated synthetic fixture), large project at both job levels
   (`.local-projects/zod` or `trpc` when present), and a repeated-execution
   series to observe run-to-run peak-RSS stability.
4. Measures wall time and peak RSS per run with the same tested `/usr/bin/time`
   wrapper as `scripts/real-projects/measure-project.ts` (`-l` bytes on macOS,
   `-v` KiB on Linux; `unavailable` elsewhere). Final RSS of a short-lived
   process is not externally observable and is always reported `unavailable`.
5. Writes per-run records and per-scenario medians/worst peaks to
   `.bench/allocators/results.json` and `summary.md`.

Interpretation rules: compare median wall time and median/worst peak RSS over
at least 5 post-warmup runs, with `--jobs 1` and parallel results kept
separate. Do not change the default allocator without a repeatable ≥5% median
time win (no material RSS regression) or ≥10% peak-RSS win (no material time
regression) on real projects.

## Complexity Regression Suite (`complexity-regression.ts`)

Guards against algorithmic (big-O) regressions rather than wall-clock drift.
It generates synthetic projects at multiple sizes under `target/complexity-gen/`
(never committed), runs the release `surge` binary with `SURGE_TIMINGS=1` at
`--jobs 1`, parses the instrumentation counters from stderr, and classifies
each tracked counter's growth as constant / ~linear / superlinear via the tail
exponent between the two largest sizes.

```bash
pnpm bench:complexity                       # builds the release CLI, runs everything
pnpm bench:complexity -- --skipBuild        # reuse target/release/surge
pnpm bench:complexity -- --json             # machine-readable output
pnpm bench:complexity -- --case union       # substring filter over case names
pnpm bench:complexity -- --sizes 32,64,128  # override sizes for quick iteration
```

Cases and their counter proxies:

- **shared-checker-options** (module graph, growing module count, fixed
  node_modules deps): `dependency_declaration_table_clone_count` and
  `generated_default_lib_table_clone_count` must stay exactly `0`;
  `type_declaration_table_clone_count` stays constant;
  `module_analysis_total_calls` stays linear.
- **serial-context-reuse** (same projects, `--jobs 1`):
  `symbol_table_clone_count` / `module_export_table_clone_count` /
  `expression_check_count` stay linear in file count.
- **union-scaling** (flat/duplicated/nested/canonical literal-alias unions plus
  a discriminated union routed through a switch and n conditional-expression
  unions): `union_type_clone_count` and `union_type_payload_alloc_count` stay
  linear; `union_type_alloc_count` and `union_type_payload_deep_clone_count`
  stay `0`.
- **overload-scaling** (growing interface overload group probed by fixed calls,
  plus n calls against small fixed groups with callbacks and duplicate
  signatures): `overload_array_alloc_count`,
  `interface_member_declaration_visit_count`, and `expression_check_count` stay
  linear; `overload_group_create_count` stays constant.
- **inheritance-scaling** (n-deep extends chain and n-wide multi-extends
  interface with repeated/overridden properties, fixed property probes):
  `interface_member_declaration_visit_count` and
  `interface_own_property_map_alloc_count` stay linear;
  `property_lookup_count` stays constant.
- **determinism**: the CLI runs twice (fresh processes, `--jobs 1`) over
  `tests/compat-projects/complexity-zod-shaped-determinism/` and — when
  present — `.local-projects/zod`, asserting identical SHA-256 of stdout.

The suite exits non-zero when a zero-expected counter becomes nonzero, a
constant/linear-expected counter classifies as superlinear, a synthetic
project unexpectedly produces diagnostics, or a determinism check fails. Wall
time is displayed per run but never gated, and there are no wall-clock
assertions anywhere in the suite. Companion tests live in
`crates/surge-ts-checker/tests/determinism.rs` (byte-identical diagnostics
across fresh in-process runs) and
`tests/checker/complexity_regression/` (dependency `.d.ts`
lexical-scope resolution, mirroring
`tests/compat-projects/complexity-dependency-lexical-scope/`).

Harness unit tests (generator determinism, stderr counter parsing, growth
classification, gating) run as part of `pnpm run bench:test` and need no cargo
build.
