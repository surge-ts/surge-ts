# Corpora check time on Windows — profile, two landed changes, two rejected designs

> **Historical engineering report — a point-in-time record.** It documents one
> investigation as it stood on 2026-10-17 and does **not** describe current
> behavior, current counts, or current performance. For the current state see
> [CURRENT_STATUS.md](../../CURRENT_STATUS.md); for benchmark methodology see
> [BENCHMARKS.md](../../BENCHMARKS.md).

Goal: bring surge's corpora check time closer to ts-rust (tsc-rs 0.7.0) without
changing any diagnostic. The session brief listed three gaps: per-module value
collection repeated across passes, generic body returns re-checked per call,
and written types re-resolved per use site. The ts-rust timings in that brief
(trpc 1.5 s, drizzle-orm 1.8 s, zod 0.6 s) were measured elsewhere and are not
re-measured here.

Everything below was measured on Windows 11, an Intel i7-12700K (8 P-cores /
16 threads plus 4 E-cores), 32 GB, rustc 1.99.1, release profile (fat LTO). The
base is `b620eccb`, built in a clean `git worktree`. Other Claude sessions were
building and testing on the same machine throughout.

## Measuring on this machine

- **Pinning is mandatory.** Unpinned, the same binary ran zod in 39–81 s and
  drizzle-orm in 40–91 s: a background process on a hybrid CPU lands on
  E-cores or under EcoQoS. Each benchmarked child was started with
  `HIGH_PRIORITY_CLASS`, affinity `0xFFFF` (logical 0–15, the P-core threads),
  and power throttling off (`SetProcessInformation(ProcessPowerThrottling)`
  with `EXECUTION_SPEED` controlled and cleared). With that, base against base
  on zod came out at −0.4% wall and +0.5% CPU (3 interleaved reps after a
  warm-up pair). Process CPU time (`GetProcessTimes`) was recorded next to wall
  time because it moves less with machine load.
- **Profiling without admin.** WPR/xperf need an elevated shell
  (`0xc5585011`). A small sampler worked instead: suspend each thread,
  `GetThreadContext`, `StackWalkEx` with dbghelp's own function-table callbacks,
  and `SymFromInlineContextW` for inline frames. That gives about 200 samples/s
  across threads on a binary built with `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`
  in its own target directory. Two details mattered: `SYMBOL_INFOW.SizeOfStruct`
  must be 88 (the `Name[1]` layout rounded to 8), and the thread snapshot must
  be refreshed rarely (every 0.5 s), not on every sample. Waiting threads
  (leaf `NtWaitFor*`) are dropped before analysis.
- **Windows-only drift.** At the base, zod reports 68 `TS7022`/`TS7023` on
  recursive getters that tsc does not, and the oracle sweep is 916/922 (the
  six failures are JavaScript/JSDoc presets). Every comparison below is A/B on
  this machine, so the drift is present on both sides.

## Profile at the base

Active samples (waiting threads excluded), the line-tables binary at `b620eccb`:

| | zod (6,189 samples) | trpc (21,613 samples) |
| --- | --- | --- |
| check phase (`check_program_files_serial`) | 55.0% | 48.8% |
| `collect_exportable_value_symbols` | 47.7% | 30.9% |
| … of which `build_module_local_values` | 19.7% | 9.7% |
| `checked_body_return` | 2.3% | 47.5% |
| `resolve_parsed_type` | 82.3% | 49.9% |
| `resolve_interface` | 74.6% | 36.6% |
| `Project::prepare` | 0.3% | 25.1% |

Both corpora check serially (`--jobs auto`). trpc's `checked_body_return` comes
34.7% from `instantiate_function_type_with_substitution` and 13.9% from
`LazyBodyReturn::resolve`. trpc's `Project::prepare` share is mostly
`ZwCreateFile` under `host_fs::read_to_string` in a cold, sampled run. An
unsampled `--timings` run spends 3.2 s in `import_graph_expansion` and 90.1 s
in `checking` of 94.5 s, so it was not pursued.

## Gap 1: reusing the module-local value table in the check phase — blocked

The proposal was that `check_program_file`'s `validation_symbols` and
`build_module_local_values`'s table for the same file are the same result, so
one could reuse the other. A trace probe (`SURGE_TRACE_VALUE_TABLES`, throwaway
worktree) printed every own entry of both tables. Each entry was keyed by file
and name and fingerprinted by variant, normalized id/display, reference
arguments to depth 2, and sorted property names, with digit runs collapsed so
fresh ids compare equal. Result on zod:

- 415 files have both tables; **196 differ**; 776 of 4,209 entries differ.
- 744 of those are a lazy-initializer reference on the module-local side
  against an eager type on the check side. `build_module_local_values` runs
  before `set_check_phase(true)`, so `lazy_origin` defers every untyped
  initializer.
- The other 32 are module-local `Any` where the check side resolves a type,
  plus function payload drift.

The two calls also differ in their seeds (import bindings + analysis symbols
against the full per-file symbol table), in the type declarations the shadow
context reads (the `local_type_declarations` argument is ignored in favor of
`ctx.type_declarations`), and in which context generation they run under.
Sharing the tables would be the cross-pass reuse the guardrails forbid.

## Gap 2: per-call generic body returns — not attempted

`instantiated_body_return` reads the body once per call under that call's
bindings. tsgo computes the return once over the type parameters and
instantiates it. The only type-level substitution surge has is
`surge_ts_types::type_variable::substitute_variable`, and it answers `Unknown`
for objects with properties, functions, and references whose arguments name
the variable. Doing gap 2 means building a full `instantiateType` first.

## Gap 3: repeated named-type expansion — two changes landed

A trace probe (`SURGE_TRACE_GENERIC_EXPANSIONS`) logged every generic expansion
that missed the caches: memo window, declaration, argument and substitution
fingerprints, and why the result was not reused. On zod there were 1,428,752
expansions. Two findings were actionable.

- **Union- and signature-bodied aliases had stopped being interned.** The early
  return that names such an expansion by its alias display (`4a651e78`, which
  says interning is untouched) skipped `intern_instantiation`. So
  `ParseReturnType<T>`-style aliases were re-expanded at every concrete
  reference, 83,807 times in one memo window for `ParseReturnType` alone.
  **`d5bbbbd7`** interns the untagged expansion under the existing conditions
  and names a short-circuit hit the way a fresh expansion is named.
- **Instantiation buckets were scanned with deep equality.** Buckets hold up to
  4,096 entries. `lookup_instantiation`, `get_persistent_generic_resolution`,
  `intern_instantiation` and their session counterparts compared every entry's
  `Vec<Type>` structurally (`Type::eq` was zod's top self-time frame at 6.5%).
  **`4ed93925`** stores an equality-consistent argument digest
  (`type_conflict_digest` per argument) on each entry and compares it first.

Measured together against the base, pinned, 3 interleaved reps:

| corpus | wall | CPU |
| --- | --- | --- |
| ts-pattern | −41.0% | −41.7% |
| drizzle-orm | −11.0% | −10.6% |
| tanstack-query | −6.7% | −7.1% |
| zod | −3.7% | −4.7% |
| trpc | −1.6% | −1.9% |
| ky, ofetch, zustand | within noise | within noise |

`--diagnosticStyle tsc --pretty false` output was byte-identical on all 8
corpora. `oracle:sweep -- --all --maxDiagnostics 200` was 916/922 for both
binaries, with identical failure and drift lines. The first change alone (an
unpinned run, so noisy) showed no clear gain. Its counters: 25k more intern
inserts, 9.6k fewer generic misses, 94k fewer named-type hits on zod.

What the same trace showed but nothing here addresses:

- Before the check phase, expansions under `collect_exportable_value_symbols`'s
  shadow context that end `had_error` are 48% of top-level expansion time on
  zod (trace build). They are concentrated in `v3/types.ts`,
  `v4/classic/schemas.ts`, and one `FlexibleSchema` expansion in the `ai`
  package's `index.d.ts`. That expansion runs from
  `packages/integration/fixtures/ai-sdk/index.ts` and costs 10.9 s and 14.7 s in
  stages 5 and 6 (trace build) against 0.6 s in the check phase. That 17-line
  file alone takes 9.1 s. Degraded results may not be cached program-wide.
- Open-scope alias instantiations in the check phase (103k same-window repeats)
  are outside the signature-context tier, which covers interfaces only.

## Rejected: skipping the binding passes' eager call/new inference

`collect_exportable_value_symbols_from_statement` infers an unannotated
`Call`/`PropertyCall`/`New` initializer eagerly and then replaces it with
`lazy_initializer_reference` whenever the result is untyped. A trace
(`SURGE_TRACE_EAGER_VALUES`) found about 2,800 such discarded inferences on
zod, mostly in stage 5 (export tables, 1,548, 3.2 s) and stage 6 (module-local
values, 718, 3.7 s), in a non-LTO trace build. Going straight to the lazy
reference was tried behind `SURGE_SKIP_PRECHECK_EAGER_CALLS` on `4ed93925`:

| variant | result |
| --- | --- |
| every binding-pass site | zod CPU −15%, but 4 new diagnostics tsc does not report: `v3/tests/error.test.ts` TS2339 ×2, `v3/tests/language-server.source.ts` TS2322 and TS2353 |
| module-local value builds only | the same 4 zod diagnostics, so they come from the tables lazy body returns read |
| export tables only | zod identical (−5% wall), tanstack-query −10%, but trpc changes 22 output lines (TS2532/TS4023 disappear, `IntersectionError` TS2339 appear) and drizzle-orm is +33% slower with +350 MB peak |

The eager attempt matters even when its result is discarded. Importers and
lazy readers get different types without it, and the cache warm-up it does
shifts later work. The experiment was not committed.
