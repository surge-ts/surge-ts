# Benchmarks

How `surge-ts` performance is measured and how to reproduce a measurement.
Every number is tied to one workload, one machine, and one commit — none of it
is a universal compiler comparison. Comparisons against other tools or other
configurations must be re-measured on the same machine in the same session.

The current measurement, together with the correctness gates taken at the same
commit, lives in
[CURRENT_STATUS.md § Current performance state](CURRENT_STATUS.md#current-performance-state).
Earlier recorded runs (2026-07-16, 2026-09-01) and the stage-by-stage history
were removed on 2026-10-17; `git show e5fc8b09:BENCHMARKS.md` has them, and the
investigations behind them are in [docs/perf/](docs/perf/).

## Methodology

- **Cold process, warm filesystem cache.** Each run is a fresh `surge`
  process; the OS file cache is warm from previous runs. There is no
  incremental or persistent mode in surge-ts — every run is a full project
  check, so these numbers are full-check numbers.
- **Median of at least 3 runs.** Wall time is reported as the median of 3+
  cold-process runs (the benchmark harness defaults to 5 iterations plus 1
  warmup per configuration). Single runs are never published.
- **Interleaved A/B runs for memory comparisons.** Peak-RSS on this workload
  varies ±30–50% run-to-run. Comparing a "before" batch against an "after"
  batch measured at different times is meaningless at that noise level;
  alternate A/B/A/B in one session and compare medians. The
  `phys_footprint`-based peak (reported by `--extendedDiagnostics`,
  `--memoryReport`, and `--reportJson`) is the Activity-Monitor-comparable
  figure on macOS; deterministic instrumentation counters (`--timings`,
  payload counts) are the stable cross-run signal.
- **Same binary, same allocator, same profile.** Wall time and memory from
  different allocators, build profiles, or binaries must not be naively
  compared — allocator choice alone shifts both. Record the allocator with
  every number; `pnpm bench:allocators` exists precisely to make allocator
  comparisons controlled.
- **Diagnostics are part of the measurement.** A speed number is only valid
  alongside an unchanged diagnostic surface; runs are paired with the
  SHA-256 of the full diagnostic output (it must be byte-identical across
  runs and across `--jobs` levels) and with oracle drift checks.

## Reproduction

1. Clone this repository and install the JS tooling:

   ```bash
   pnpm install
   ```

2. Place a tRPC checkout at `.local-projects/trpc`, pinned to the commit of
   the run you are reproducing (the checkout is not distributed with this
   repository). For the commit CURRENT_STATUS.md records:

   ```bash
   git clone https://github.com/trpc/trpc .local-projects/trpc
   git -C .local-projects/trpc checkout dfbafa8ef178a5a3d23ef9461caa9494b3ef7f95
   ```

   Confirm which commit is actually on disk (`git -C .local-projects/trpc
   rev-parse HEAD`) before recording a number — different checkouts are
   different workloads (`tsc` itself reports a different diagnostic count on
   them).

3. Build the release CLI:

   ```bash
   cargo build --release -p surge-ts-cli
   ```

4. Run the measurement harness:

   ```bash
   pnpm real:trpc
   ```

   This runs `scripts/real-projects/measure-project.ts --project
   .local-projects/trpc --name trpc --allowMissing`, which: compares
   diagnostics against the TypeScript oracle (`oracle:compare`, text + JSON),
   rebuilds the release binary, samples peak RSS per output mode under
   `/usr/bin/time`, runs the compiler benchmark at each requested job level
   (default `--rustJobs 1,auto`; 5 iterations + 1 warmup each), and writes all
   artifacts plus a `measurement.md` summary under
   `.bench/real-projects/trpc/`. Useful options: `--maxDiagnostics <N>`
   (default 500), `--rustJobs <list>`, `--outDir <dir>`; `--allowMissing`
   makes a missing checkout a recorded no-op instead of a failure.

   For a single manual timing run without the harness:

   ```bash
   target/release/surge --project .local-projects/trpc/tsconfig.json --jobs 1 --extendedDiagnostics
   ```

Expect different absolute numbers on different hardware, OS versions, or
toolchains; the fixture commit pin is what makes the workload itself
comparable.

## Related harnesses

- `pnpm bench:compilers` — wall-clock comparison of `tsc` (TS 6), `tsgo`
  (native TS 7), and the release `surge` binary over the same tsconfig, with
  diagnostic-drift reporting against `tsgo` (TS 7); `tsc` (TS 6) is kept only
  as the speed and memory reference.
- `pnpm bench:allocators` — builds the `surge` binary with each supported
  global allocator (system, mimalloc, jemalloc, snmalloc) and runs a
  wall-time/peak-RSS scenario matrix.
- `pnpm bench:archive` — wraps benchmark and real-project runs and archives
  their raw output plus a summary under `.bench/runs/<timestamp>/` for
  before/after comparison.
- `pnpm bench:complexity` — deterministic complexity-scaling regression
  suite: generates synthetic projects at multiple sizes and gates on
  instrumentation-counter growth (constant/linear/superlinear), not wall
  time.

Details for all four live in [scripts/bench/README.md](scripts/bench/README.md).
