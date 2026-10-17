# TypeScript Oracle Comparison

This workspace keeps a pinned root `typescript` dev dependency and a small
comparison harness for measuring `surge-ts` against the TypeScript
compiler.

The Node toolchain is dev-only. Rust crates do not depend on Node tooling, and
`cargo test` does not require `pnpm install`.

## Lockfile policy

- The repository commits `pnpm-lock.yaml`.
- `node_modules/` stays ignored.
- `.local-projects/` stays ignored.
- No yarn lockfile is used.
- The TypeScript version is pinned in the root `package.json`; do not switch it
  to `latest`.

## Install

```bash
pnpm install
```

## Run

Compare a committed fixture:

```bash
pnpm run oracle:compare -- --project tests/compat-projects/generics-basic/tsconfig.json
```

Compare one of the built-in presets:

```bash
pnpm run oracle:compare -- --project generics-basic
```

Compare a single standalone source file using TypeScript-like CLI behavior:

```bash
pnpm run oracle:compare -- --file examples/basic.ts
pnpm run oracle:compare -- --file examples/assignment.ts
```

Compare a single standalone source file while explicitly bypassing TS5112
and running semantic checking:

```bash
pnpm run oracle:compare -- --file examples/basic.ts --ignoreConfig
```

Compare a project while suppressing external package missing-module errors:

```bash
pnpm run oracle:compare -- --project package-imports --stubExternalModules
```
*Note: `--stubExternalModules` is a surge-ts-only compatibility flag. The oracle does not pass it to TypeScript. In this mode, surge-ts suppresses non-relative missing-module diagnostics, including TS2307 and the side-effect-import TS2882 form, while TypeScript still reports its normal diagnostics.*

Compare a project while requesting deterministic parallel project checking:

```bash
pnpm run oracle:compare -- --project tests/compat-projects/parallel-ordering-basic/tsconfig.json --rustJobs 4
```
`--rustJobs` only affects the `surge-ts` command. It does not change the `tsc` baseline or the oracle comparison rules.
The rendered comparison also prints the exact `surge-ts` command and the explicit job count so stale-binary and wrong-workspace confusion is easier to spot.

The `package-imports` fixture pins TypeScript 6.0.3 behavior for unresolved
package imports: ordinary imports and re-exports remain TS2307, while a bare
side-effect import such as `import "reflect-metadata";` is TS2882. This is a
diagnostic-priority parity check only; it is not package resolution.

Compare the declaration-ingestion fixture through the preset system:

```bash
pnpm run oracle:compare -- --project declarations-basic
pnpm run oracle:compare -- --project declarations-hardening
```

The declaration presets are pinned compatibility fixtures, not a claim that the oracle handles full package discovery or full declaration merging. Supported: bare packages via types, typings, index.d.ts fallback.


Compare a disposable local project:

```bash
pnpm run oracle:compare -- --project .local-projects/<project>/tsconfig.json --maxDiagnostics 200
```

Run the parser and comparison tests:

```bash
pnpm run oracle:test
```

## Sweeping multiple targets

`oracle:compare` checks one target at a time. `oracle:sweep` runs the same
oracle comparison across many targets and prints a compact, regression-oriented
summary. It reuses the exact comparison from `compare-tsc.ts` (each target runs
through `oracle:compare --json`) and never adds classifiers, suppresses
mismatches, or rewrites fixture expectations.

It is not limited to the registered presets: a target can be a preset, an
explicit `tsconfig.json` / project directory, a single source file, or every
`tsconfig.json` discovered under a directory.

List the selected targets and exit:

```bash
pnpm run oracle:sweep -- --list --all
pnpm run oracle:sweep -- --list --filter node-protocol
pnpm run oracle:sweep -- --list --discover tests/compat-projects
```

Run a targeted group, an arbitrary project, or everything, optionally in
parallel:

```bash
pnpm run oracle:sweep -- --filter node-protocol --maxDiagnostics 200
pnpm run oracle:sweep -- --all --exclude diagnostics-pack --maxDiagnostics 200
pnpm run oracle:sweep -- --all --jobs 4 --maxDiagnostics 200
pnpm run oracle:sweep -- --project .local-projects/app/tsconfig.json --maxDiagnostics 200
pnpm run oracle:sweep -- --file examples/basic.ts
pnpm run oracle:sweep -- --discover tests/compat-projects --jobs 4 --maxDiagnostics 200
```

Target sources:

- `--all` selects every registered preset.
- `--filter <substring>` keeps presets and discovered targets whose name
  includes the substring (repeatable; works without `--all`).
- `--exclude <substring>` drops any target whose name includes the substring
  (repeatable).
- `--project <path|dir|preset>` adds an explicit project; explicit projects are
  always run and are not removed by `--filter` (repeatable).
- `--file <source.ts>` adds a single source file (repeatable).
- `--discover <dir>` recursively adds every `tsconfig.json` under a directory,
  skipping `node_modules` (repeatable).
- Targets are deduplicated by resolved path, so discovering the preset tree does
  not double-run presets. Order is deterministic: presets in registry order,
  then explicit targets in argument order, then discovered targets sorted by
  name.
- `--list` prints the selected names and exits; bare `--list` (no other source)
  lists all presets.
- Running with no `--all`, `--filter`, `--list`, `--project`, `--file`, or
  `--discover` prints usage instead of starting a full sweep.
- A selection that matches nothing exits non-zero.

Default gate and drift:

- A preset fails on diagnostic code-count mismatch or file/code/line mismatch.
- Message-text drift and span/column drift are reported but do not fail the run
  unless you pass `--strictMessages` or `--strictSpans`.
- The summary always surfaces `messageDriftOnly` and `spanDriftOnly` counts so
  lower-priority drift stays visible without gating development.

Each preset prints one compact line, for example:

```txt
PASS node-protocol-buffer-basic ts=1 rust=1 onlyTsc=0 onlyRust=0 fileCodeLine=yes message=yes span=yes elapsed=312ms
```

Failing presets print their code/file/line buckets underneath; `--verbose`
prints the full per-preset oracle output. `--json` emits a stable object with
`selected`, `skipped`, per-preset `results`, and an aggregate `summary`
(including the final `exitCode`).

## What it does

- Project mode runs:
  ```txt
  pnpm exec tsc --noEmit --pretty false --project <tsconfig>
  cargo run ... -- --project <tsconfig> --format json [--jobs <n>]
  ```
- File mode without `--ignoreConfig` runs:
  ```txt
  pnpm exec tsc --noEmit --pretty false <file>
  cargo run ... -- --format json <file>
  ```
  and may report TS5112 when a `tsconfig.json` is present.
- File mode with `--ignoreConfig` runs:
  ```txt
  pnpm exec tsc --noEmit --pretty false --ignoreConfig <file>
  cargo run ... -- --format json --ignoreConfig <file>
  ```
  and is the explicit standalone semantic comparison mode.
- The oracle never secretly adds `--ignoreConfig`.
- Normalizes both diagnostic streams to code, file name, line, and column when
  available.
- Diagnostic fingerprints used in tests can also include message text when needed for deterministic equality checks.
- Compares code counts first, then `(fileName, code)` counts, then
  `(fileName, code, line)` where both sides have line data.
- Reports message parity: diagnostics that share an exact
  `(fileName, code, line, column)` are paired, and any remaining message-text
  difference is listed with the `tsc` and `surge-ts` text side by side.
  Pairs whose spans differ are left to the span-level levels, so this section
  isolates pure message-text drift.

## What it does not do

- It does not require exact message parity by default. Message differences are
  reported and informational unless you pass `--strictMessages`, which exits
  with code 1 when any same-location message text differs from `tsc`.
- It does not require exact span parity.
- It does not add *any* checker behavior. The harness only runs the two
  compilers and compares their output; module resolution, declaration
  semantics, `lib.d.ts` loading, `@types` discovery, and declaration merging
  are whatever the checker itself implements. For what that currently covers,
  see [CURRENT_STATUS.md](../../CURRENT_STATUS.md) and
  [PUBLIC_API.md](../../PUBLIC_API.md) — do not read this list as a checker
  support statement.
- File mode currently only accepts `.ts` files. Project mode is still the
  preferred oracle for multi-file compatibility checks, and file mode may drift
  from project mode because single-file TypeScript runs use default compiler
  options unless you add explicit flags later.
- It is a measurement tool, not a claim that the checker fully matches
  TypeScript.

## Common Mistake

This is invalid:

```bash
pnpm run oracle:compare -- --project examples/basic.ts
```

`--project` expects a preset name or `tsconfig.json` path. Use `--file` for a
single source file. This prevents TypeScript from treating `.ts` files as
`tsconfig` inputs and emitting misleading config diagnostics.

## Output levels

- Level 1: by code
- Level 2: by file and code
- Level 3: by file, code, and line when available
- Message parity: by file, code, line, and column, comparing message text

The default comparison prints all levels when possible. Code/file mismatches are
informational unless you pass `--failOnMismatch` or `--strictCodes`; message-text
mismatches are informational unless you pass `--strictMessages`.
