# TypeScript-Go Upstream Testdata

This directory tracks a small subset of upstream test cases from:

https://github.com/microsoft/typescript-go/tree/main/testdata

The full upstream testdata suite is intentionally not vendored.

Only small cases are copied here when the current checker can meaningfully run them.

Rules:

- Vendored cases must be copied from the upstream repository unchanged.
- Every vendored case records its upstream path and baseline in
  `manifest.toml` under `[[case]]`; `tests/upstream.test.ts` fails when a file
  under `cases/` has no entry or an entry has no file.
- Custom local tests belong in `tests/smoke` or `tests/checker`, not in this
  directory.
- Do not rewrite upstream tests to fit this checker.

## How cases run

`tests/upstream.test.ts` runs every vendored case through the fixture harness
([tests/README.md](../../README.md)): the case's own `// @filename:` markers
split it into files and its compiler-option directives become the scratch
project's tsconfig, so an upstream `@strict`, `@module`, `@allowJs`, … applies
as written. surge and the pinned tsc 7.0.2 check that project and must report
the same diagnostics. The expectation is tsc run live, not the upstream baseline
file, and a fixture that upstream runs once per option variant is run once with
the first variant.

## Pending records

`[[pending]]` entries are upstream cases that are *not* vendored. Each carries
the upstream baseline's codes (`baseline_diagnostics`) and a `reason` naming
what surge reported. **Those reasons were measured by the Rust smoke harness
retired at `110bbac4` (2026-09-28)**, which ignored compiler-option headers and
did no package resolution; they are a record of the gap at that point, not of
current behaviour. To take a case up, vendor its file, move its entry to
`[[case]]`, and let the live comparison decide.

## Scope

The upstream fixture subset here is intentionally small and is not a
baseline-compatibility claim; current support is recorded in
[CURRENT_STATUS.md](../../../CURRENT_STATUS.md). New upstream-style cases should
map to a real checker or parser emission path, not merely to a code that exists
in the diagnostic catalog.
