# surge-ts

`surge-ts` is a TypeScript type checker written in Rust, focused on
`tsc`-compatible diagnostics for `noEmit`-style project checking. It is not a
full TypeScript compiler and does not claim full TypeScript compatibility:
compatibility is measured, per feature and per project, against the upstream
compiler as an oracle. The engineering priorities are practical checking
performance, bounded memory use, deterministic output (identical diagnostics
across repeated runs and across serial/parallel execution), and validation on
real-world projects rather than synthetic claims.

The workspace ships an embeddable library (`surge-ts`, with the lower-level
`surge-ts-checker`) and a CLI (`surge-ts-cli`, binary name `surge`). The stable
API surface and the exact-parity feature list live in
[PUBLIC_API.md](PUBLIC_API.md).

> **Current state:** [CURRENT_STATUS.md](CURRENT_STATUS.md) is the canonical
> answer to "what is true right now" — verified gate results, the real-project
> parity matrix, current known limitations, feature-gated behavior, and the
> latest benchmark measurement with its caveats. Start there.

## Quick start

Build the CLI (release profile — debug builds are much slower and not
representative):

```bash
cargo build --release -p surge-ts-cli
```

Check a project the way `tsc --noEmit` would:

```bash
target/release/surge --project path/to/tsconfig.json
```

```text
src/index.ts(11,7): error TS2322: Type 'string | undefined' is not assignable to type 'string'.
src/index.ts(19,12): error TS2339: Property 'missing' does not exist on type 'User'.
```

The exit code is `0` when there are no diagnostics and `2` when there are.
A single `.ts` file can also be passed positionally instead of
`--project`. Useful run-level flags: `--jobs <auto|N>` (worker threads; `auto`
is the default), `--maxDiagnostics <N>` (cap displayed diagnostics), and
`--pretty <true|false|auto>` (tsc-style code frames).

Machine-readable diagnostics on stdout:

```bash
target/release/surge --project path/to/tsconfig.json --format json
```

```json
{
  "diagnostics": [
    {
      "code": "TS2322",
      "fileName": "src/index.ts",
      "message": "Type 'string | undefined' is not assignable to type 'string'.",
      "span": { "start": 201, "end": 208 },
      "line": 11,
      "column": 7
    }
  ]
}
```

(Abbreviated to the first diagnostic; the array carries every diagnostic.)

Run statistics on stderr (diagnostics output on stdout is unchanged):

```bash
target/release/surge --project path/to/tsconfig.json --extendedDiagnostics
```

```text
Extended diagnostics:
  files:                                 64
    source files:                         1
    dependency declaration files:         0
    default lib files:                   63
  diagnostics:                            5
  jobs:                                auto
  allocator:                         system
  config/project loading:           0.398ms
  ...
  checking:                        18.902ms
  total:                           22.772ms
  peak physical footprint:         40.1 MiB
  finish physical footprint:       40.1 MiB
  peak rss:                        43.5 MiB
```

A versioned machine-readable run report (`schemaVersion: 1`), written to a
file:

```bash
target/release/surge --project path/to/tsconfig.json --reportJson report.json
```

```json
{
  "schemaVersion": 1,
  "summary": {
    "files": 64,
    "sourceFiles": 1,
    "dependencyDeclarationFiles": 0,
    "defaultLibFiles": 63,
    "diagnostics": 5,
    "wallTimeMs": 29.521,
    "jobs": "auto",
    "allocator": "system"
  },
  "phases": { "checkingMs": 23.965, "totalMs": 29.521 },
  "memory": {
    "peakPhysicalBytes": 43598280,
    "finishPhysicalBytes": 43598280,
    "peakRssBytes": 47153152
  }
}
```

(The `phases` object is abbreviated here; the full schema, including
`--memoryReport` and the guarantee that reporting flags never change the
diagnostics output, is documented in [PUBLIC_API.md](PUBLIC_API.md).)

## Status at a glance

The current gate results — workspace tests, the oracle preset sweep under the
normal gate and both strict flags, and every real-project corpus — are recorded
in [CURRENT_STATUS.md § Verification snapshot](CURRENT_STATUS.md#verification-snapshot)
together with the commit they were measured at. They are volatile, so they live
there and nowhere else.

The normal gate is diagnostic code-count and file/code/line parity against the
upstream compiler. Message text and span/column are separate, non-gating
dimensions: a preset can record drift without failing CI, and the drift that
currently exists is listed in [STRICT_DRIFT_INVENTORY.md](STRICT_DRIFT_INVENTORY.md).

Projects where `tsc` reports zero diagnostics are pinned as **false-positive
corpora**, with gates of different strength: `pnpm run real:ky:test` asserts
exact 0/0, so any surge diagnostic fails it, while `pnpm run real:unnamed:test`
asserts a count ceiling (a ratchet that only moves down). Both skip cleanly
when the checkout or the `typescript` package is absent, and a skipped gate is
not a passing gate. What this evidence does *not* establish is full TypeScript
compatibility — projects outside the covered surface can and do drift, and the
known gaps are tracked openly in [CURRENT_STATUS.md](CURRENT_STATUS.md#known-limitations);
the corpora are listed in [REAL_PROJECT_COMPAT.md](REAL_PROJECT_COMPAT.md). tRPC is a measured
workload, never a parity claim.

## Performance

The last recorded measurement — one workload (the tRPC monorepo) on one
machine at one commit — is in
[CURRENT_STATUS.md § Current performance state](CURRENT_STATUS.md#current-performance-state),
with its caveats. It is not a compiler comparison and does not transfer across
projects, hardware, allocators, or build profiles; only interleaved A/B pairs
taken in one session compare, and peak RSS on that workload varies ±30–50% run
to run without that discipline.

What *is* a property rather than a measurement: repeated runs render
byte-identical diagnostics, and `--jobs 1` and `--jobs auto` produce identical
output. Methodology and reproduction steps: [BENCHMARKS.md](BENCHMARKS.md).

## Architecture overview

The workspace is split into small crates with stable facades: parsing
(`surge-ts-syntax`), the core type representation (`surge-ts-types`), semantic
checking (`surge-ts-checker`), diagnostics catalog/rendering
(`surge-ts-diagnostics`), tsconfig loading (`surge-ts-config`), the embeddable
umbrella crate (`surge-ts`), and the CLI (`surge-ts-cli`). Project checking is
a fixed phase pipeline — config load, file discovery, import-graph expansion
(including physical `lib*.d.ts` loading from the local `typescript` package),
parse, module analysis/binding, per-file checking (serial or parallel), and
rendering.

Two design decisions carry most of the performance and determinism weight.
First, each run owns a canonical `ProgramTypeStore` that interns structural
type payloads (function types, union member lists, property maps, parameter
lists) so identical types are shared write-once `Arc`s instead of repeated
allocations; type IDs embed a program-owner tag and never cross runs, and the
store is torn down at end of run so embedders do not retain the type graph.
Second, checker options and module-resolution tables are shared immutably for
the whole run (`Arc<CheckerOptions>`) — contexts are never deep-cloned per
module, and every cross-module cache must key on declaration identity,
arguments, and environment. The rules that keep this fast and deterministic
are written down as enforceable invariants: see
[ARCHITECTURE.md](ARCHITECTURE.md) and
[docs/PERFORMANCE_INVARIANTS.md](docs/PERFORMANCE_INVARIANTS.md).

## Scope and non-goals

In scope: `tsconfig.json` project checking with `tsc`-compatible diagnostics,
physical `lib*.d.ts` loading from the local `typescript` package,
declaration-side module resolution (`paths`, `baseUrl`, package `exports` /
`imports`, `typesVersions`, self-name, `types` / `typeRoots`, `@types`
discovery, `/// <reference types>`), JSON modules under `resolveJsonModule`,
and deterministic serial or parallel checking.

Explicitly **not** goals: emit, transforms, declaration emit, a language
service, incremental or watch checking, building referenced projects, full runtime/JS
package resolution, and full `lib.d.ts` / DOM / Node / React parity. The
current limitation list — reconstructed from probes rather than inherited from
old notes — is in [CURRENT_STATUS.md](CURRENT_STATUS.md#known-limitations).

## Documentation map

| Question | Document |
| --- | --- |
| What is true right now? | **[CURRENT_STATUS.md](CURRENT_STATUS.md)** — canonical current state |
| What API is stable, and what is held at exact parity? | [PUBLIC_API.md](PUBLIC_API.md) |
| How does the checker work? | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Where does module resolution live, and how is it checked against tsc? | [crates/surge-ts/MODULE_RESOLUTION.md](crates/surge-ts/MODULE_RESOLUTION.md) |
| Real-project corpora and how to run them | [REAL_PROJECT_COMPAT.md](REAL_PROJECT_COMPAT.md) |
| Remaining message/span drift | [STRICT_DRIFT_INVENTORY.md](STRICT_DRIFT_INVENTORY.md) |
| Benchmark methodology and reproduction | [BENCHMARKS.md](BENCHMARKS.md) |
| Rules for performance-sensitive changes | [docs/PERFORMANCE_INVARIANTS.md](docs/PERFORMANCE_INVARIANTS.md) |
| Optimization investigations (point-in-time records) | [docs/perf/](docs/perf/) |

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
