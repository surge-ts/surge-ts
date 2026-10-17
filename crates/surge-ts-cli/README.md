# surge-ts

CLI entry point for the workspace checker. `surge-ts` is a Rust-based TypeScript
noEmit compatibility checker; it aims for tsc-compatible diagnostics in
noEmit-style project checks.

The built binary is `surge` (`target/release/surge`). The crate is published
internally as `surge-ts-cli`, so `cargo` invocations use `-p surge-ts-cli`.

Diagnostics are catalog-driven and rendered through the shared diagnostics crate.

> This file documents the CLI surface. For the current support surface, known
> gaps, and verified gate results, see
> [CURRENT_STATUS.md](../../CURRENT_STATUS.md). Any `v0.x` / `v1.x` label
> appearing in this repository is an internal milestone marker, not a release
> or crate version.

## Modes

- Single-file mode: `surge <file.ts>`
- Single-file mode (standalone): `surge --ignoreConfig <file.ts>`
- Project mode: `surge --project <tsconfig.json>`
- Project mode with deterministic parallel per-file checking: `surge --project <tsconfig.json> --jobs 4`
- Compatibility report: `surge --project <tsconfig.json> --compatReport`
- Stub external modules: `surge --project <tsconfig.json> --stubExternalModules`

`--compatReport` always prints the discovered file count. If project mode loads
zero source files, the CLI emits a custom
`surge::project-has-no-source-files` diagnostic and the report
includes a visibility warning instead of silently returning an empty
comparison. The report also includes lightweight build provenance so it is
clear whether the output came from the current workspace binary: package
version, build profile, binary path, current directory, and workspace root.

## Diagnostic output styles

The default human-readable output is **tsc-compatible**. The original
project-specific (Rust-style `error[TS....]` / ` --> `) output is preserved
behind an explicit flag, and JSON remains explicitly opt-in.

- `--diagnosticStyle <tsc|custom|json>` (alias: `--diagnostic-style`) selects the
  renderer. Default: `tsc`.
  - `tsc` — TypeScript-compiler-compatible text output (the default).
  - `custom` — the original `surge-ts` report style.
  - `json` — machine-readable diagnostics (equivalent to `--format json`).
- `--pretty <true|false|auto>` controls the multi-line `tsc` code-frame output.
  Default: `auto` (pretty when stdout is a TTY, like `tsc`).
- `--format json` continues to emit JSON (back-compat; used by the oracle
  harness). `--format text` maps to the default `tsc` style.

`--pretty false` matches `tsc`'s one-line-per-diagnostic output:

```text
src/index.ts(3,1): error TS2588: Cannot assign to 'a' because it is a constant.
```

`--pretty true` matches `tsc`'s multi-line code frame and summary footer:

```text
src/index.ts:3:1 - error TS2588: Cannot assign to 'a' because it is a constant.

3 a = 3;
  ~

Found 1 error in src/index.ts:3
```

When pretty output is enabled and color is active, ANSI escape sequences match
`tsc` (cyan file, yellow line/column, red `error`, gray code, inverse gutter,
red squiggle). Color follows the terminal by default and honors the standard
`NO_COLOR` / `FORCE_COLOR` environment variables for deterministic output.

`--showSpans` remains a debug affordance and forces the custom span renderer
even under the default `tsc` style.

`--diagnosticStyle` is independent of `--diagnosticProfile`: the former selects
how diagnostics are *rendered*, while the latter selects which diagnostics the
*checker* emits (oracle-aligned `tsc` vs. cascade-suppressing `native`).

The footer matches `tsc` exactly for the single-file/single-error case
(`Found 1 error in <file>:<line>`), the same-file multi-error case
(`Found N errors in the same file, starting at: <file>:<line>`), and the
multi-file case (`Found N errors in M files.` followed by the `Errors  Files`
table). Multi-line span underlining renders the span's starting line; rendering
every line of a multi-line span is deferred (the JSON output and oracle
comparison are unaffected).

## External Modules

By default, unresolved non-relative package imports emit TS2307.
`--stubExternalModules` suppresses non-relative TS2307 and inserts unknown type/value stubs.
This is a surge-ts-only compatibility mode.

## Declaration Files & Built-ins

Loaded `.d.ts` files from project inputs participate in semantic checking.
Module names, package entrypoints, `paths`, `/// <reference types>` and
`compilerOptions.types` resolve through the typescript-go resolver port (see
[crates/surge-ts/MODULE_RESOLUTION.md](../surge-ts/MODULE_RESOLUTION.md)); missing
configured types and reference-type directives report `TS2688`, suppressed in
declaration files by `skipLibCheck`. TypeScript 6+ does not implicitly include
every visible `@types` package when `types` is absent; see
[AUTO_TYPES.md](AUTO_TYPES.md) for `types`/`typeRoots`/`@types` semantics.

Project mode loads a **bundled, version-pinned** `lib*.d.ts` graph embedded in the binary as ambient default libs, so no `typescript` package is required. `--typescript-lib-path <DIR>` or `--physicalLibs` select an on-disk lib directory instead; `noLib: true` disables loading from any source.

`.tsx` files parse JSX syntax (elements, fragments, attributes, and `{...}`
expression containers). JSX element and prop validation is implemented and
oracle-gated: `JSX.IntrinsicElements` lookup, function-component and
imported-component props, member tags, `children`, ref-as-prop, generic-angle
disambiguation, and `jsx: react-jsx` runtime lookup through `@types/react`.

This is **not** the JSX transform. See [PUBLIC_API.md](../../PUBLIC_API.md) § 2.2
for the gated JSX surface and
[CURRENT_STATUS.md](../../CURRENT_STATUS.md#known-limitations) for the current
gaps.

## Single-file behavior

Positional file mode follows TypeScript CLI config behavior. If a `tsconfig.json` is discovered in the current working directory under the pinned policy and `--ignoreConfig` is absent, TS5112 is emitted before semantic checking.

`--ignoreConfig` intentionally bypasses config discovery and runs standalone semantic checking.

`--project` and `--ignoreConfig` cannot be combined in current policy.

Checker APIs do not emit TS5112.

Example docs:

```bash
# TypeScript-like CLI behavior; may emit TS5112
cargo run -p surge-ts-cli -- examples/basic.ts

# Standalone semantic file checking
cargo run -p surge-ts-cli -- --ignoreConfig examples/basic.ts
```

## JSON output

- `--format json` (or `--diagnosticStyle json`) prints diagnostic JSON in normal project or single-file mode.
- `--compatReport --format json` prints compatibility-report JSON.
- `--diagnosticProfile <tsc|native>` sets the diagnostic profile. The `tsc` profile strictly aligns with TypeScript's oracle baseline, while `native` aggressively suppresses noisy cascades at boundaries like `satisfies`. (Default: `tsc`)
- `--jobs` is project-mode infrastructure for deterministic per-file checking only. It keeps shared prepasses serial and merges diagnostics in loaded-file order. The default is `auto`.
- `--showSpans` is a text-mode affordance; JSON output already carries spans and,
  when available, 1-based line and column numbers.
- `--maxDiagnostics` limits rendered diagnostics in normal diagnostic mode.
- `--extendedDiagnostics`, `--memoryReport`, and `--reportJson <PATH>` emit run
  statistics; none of them changes the diagnostics output. `--timings` and
  `--rss` are hidden debug flags (equivalent to `SURGE_TIMINGS=1` /
  `SURGE_RSS=1`).
- `--compatReport` is a raw measurement surface: it reports totals, counts by code and file, parser-error grouping, loaded file counts, file-kind counts, and suppressed diagnostic totals where relevant. It does not perform semantic diagnosis. Raw parity analysis belongs in oracle output, fixtures, and implementation notes.
- The oracle comparison output prints the exact `surge-ts` command and the explicit job count when `--rustJobs` is provided.

The JSON diagnostic shape stays stable across the catalog migration:

- `code`
- `message`
- `fileName`
- `line`
- `column`
- `span`

## Workflow notes

- The CLI is pure Rust; it does not require Node tooling to build or test.
- `cargo test` does not require `pnpm install`.
