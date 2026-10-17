# Public API & Verified Features

This document is the **stable surface** of `surge-ts`: the public API you can
embed against, and the feature set that is held to **exact TypeScript (`tsc`)
parity** by a regression gate. Everything listed here is either covered by the
stable-API guarantees in code or pinned by an oracle-gated fixture — it is not a
wish list.

For current measured compatibility and known gaps, see
[`CURRENT_STATUS.md`](CURRENT_STATUS.md); for the real-project corpora, see
[`REAL_PROJECT_COMPAT.md`](REAL_PROJECT_COMPAT.md).
Anything **not** listed here should be treated as unstable or out of scope.

This document is the **contract**, not the status board. It deliberately avoids
volatile counts (preset totals, test totals, per-project diagnostic numbers,
benchmark medians) — those live in [`CURRENT_STATUS.md`](CURRENT_STATUS.md) and
are re-measured there.

---

## 1. Public API

Two layered crates:

- **`surge-ts`** — the umbrella/facade crate. Re-exports the in-memory API
  **and** adds [`Project`] for full `tsconfig.json` project checking (config
  loading, package/`paths`/reference resolution, default-lib loading, the
  import-graph fixpoint). **Depend on this** unless you only need in-memory
  checking.
- **`surge-ts-checker`** — the in-memory checking engine: the [`Checker`]
  builder over `SourceFileInput`s. Lighter dependency when you have no tsconfig.

### 1.1 Project mode (`surge-ts::Project`)

Check a whole `tsconfig.json` project the way the CLI does — but from a library:

```rust
use surge_ts::{Project, ProjectOptions};

let project = Project::load("tsconfig.json");
if project.is_empty() {
    // no input files discovered
}
let result = project.check(&ProjectOptions::default());
for d in &result.diagnostics {
    println!("{} {}: {}", d.code, d.file_name, d.message);
}
```

| Symbol | Kind | Purpose |
| --- | --- | --- |
| `Project` | struct | `load(tsconfig)` → `config()` / `config_diagnostics()` / `is_empty()` / `check(&opts)` |
| `ProjectOptions` | struct | `jobs`, `stub_external_modules`, `diagnostic_profile`, `physical_libs_requested`, `collect_timings` (all `Default`) |
| `ProjectCheckResult` | struct | `{ diagnostics, stats, sources, warnings, timings }` |
| `ProjectTimings` | struct | Per-phase durations + I/O counters (only when `collect_timings`) |
| `ProjectError` | enum | `SourceRead { path, error }` |
| `ProjectSource` | type | `(PathBuf, file_name, source_text)` — for rendering code frames |

`Project::load` does config discovery/normalization only; `Project::check` does
source reading, resolution, default-lib loading, and the type check. Strictness
and module/lib options come from the loaded config — `ProjectOptions` carries
only the run-level knobs the config does not.

The facade also re-exports the in-memory API and the config types
(`LoadedTsConfig`, `ConfigDiagnostic`, `ScriptTarget`, `TsConfigLoadOptions`),
so a project-mode embedder needs only the one `surge-ts` dependency.

### 1.2 In-memory mode (`surge-ts-checker::Checker`)

The [`Checker`] builder takes in-memory source files and returns diagnostics
plus tsc-compatibility stats. (Re-exported from `surge-ts` too.)

```rust
use surge_ts_checker::{Checker, SourceFileInput};

let result = Checker::new()
    .no_implicit_any(true)
    .check(vec![SourceFileInput {
        file_name: "index.ts".to_string(),
        source_text: "const x: number = 1;".to_string(),
    }]);

assert!(result.diagnostics.is_empty());
```

**Stable exports** (`surge_ts_checker::*`, also re-exported from `surge_ts::*`):

| Symbol | Kind | Purpose |
| --- | --- | --- |
| `Checker` | builder struct | Configure and run a check |
| `CheckResult` | type alias | `= ProgramCheckResult` (diagnostics + stats) |
| `ProgramCheckResult` | struct | `{ diagnostics, stats }` |
| `SourceFileInput` | struct | `{ file_name, source_text }` |
| `CheckerOptions` | struct | Full option set (also settable in one call) |
| `CompatibilityStats` | struct | Suppression / unresolved-module counters |
| `DiagnosticProfile` | enum | `Tsc` (default) or `Native` |
| `FileKind` | enum | Source / declaration / physical-lib classification |
| `Diagnostic`, `DiagnosticCode`, `DiagnosticCategory`, `TextSpan` | — | Read check output (code, message, span, severity) |

**`Checker` builder methods** — one per `CheckerOptions` field, so the whole
option set is reachable fluently:

| Method | Effect |
| --- | --- |
| `Checker::new()` / `Checker::default()` | Default options, `jobs = 1` |
| `.options(CheckerOptions)` / `.options_mut()` | Replace / borrow the full option set |
| `.jobs(usize)` | Worker threads for multi-file checks (clamped to ≥ 1) |
| `.no_implicit_any(bool)` | `noImplicitAny` |
| `.no_implicit_returns(bool)` | `noImplicitReturns` |
| `.no_fallthrough_cases_in_switch(bool)` | `noFallthroughCasesInSwitch` |
| `.no_implicit_override(bool)` | `noImplicitOverride` |
| `.no_property_access_from_index_signature(bool)` | `noPropertyAccessFromIndexSignature` |
| `.no_unused_locals(bool)` / `.no_unused_parameters(bool)` | unused-binding checks |
| `.no_lib(bool)` / `.skip_lib_check(bool)` | default-lib toggles |
| `.stub_external_modules(bool)` | Suppress non-relative missing-module diagnostics |
| `.types(Vec<String>)` | Effective `@types` package names |
| `.resolved_modules(HashMap<String, String>)` | Pre-resolved specifier → file map |
| `.diagnostic_profile(DiagnosticProfile)` | Select `Tsc` vs `Native` output |
| `.check(Vec<SourceFileInput>)` → `CheckResult` | Check a multi-file program |
| `.check_source(&str, &str)` → `Vec<Diagnostic>` | Check a single in-memory file |

> **Not stable:** the `lowlevel` module (default-lib loading, physical-lib
> resolution, seed catalog) and the `#[doc(hidden)]` free functions
> (`check_source`, `check_program*`). These can change without a major-version
> bump. Embedders should go through `Checker` or `Project`.

### 1.3 CLI (`surge-ts-cli`, binary `surge`)

Two input modes: a single `.ts` **file** (positional argument) or a
`tsconfig.json` **project** (`--project`).

```bash
# Single file (quick standalone oracle)
surge path/to/file.ts

# Project mode (the main compatibility path)
surge --project ./tsconfig.json
```

**Stable flags:**

| Flag | Purpose |
| --- | --- |
| `-p, --project <TSCONFIG>` | Check a tsconfig-based project |
| `--ignoreConfig` | File mode: ignore any discovered config (file-only) |
| `--showConfig` | Print the resolved config (requires `--project`) |
| `--compatReport` | Emit the compatibility-report JSON (requires `--project`) |
| `--extendedDiagnostics` | Print run statistics (files, phase times, memory) to stderr after the diagnostics (requires `--project`) |
| `--memoryReport` | Print a memory-focused report to stderr after the diagnostics (requires `--project`) |
| `--reportJson <PATH>` | Write a versioned machine-readable run report to `PATH` (requires `--project`; see §1.4) |
| `--format <json>` | Machine-readable JSON output (oracle harness format) |
| `--diagnosticStyle <tsc\|custom\|json>` | Select the renderer |
| `--pretty <true\|false\|auto>` | `tsc`-style code-frame output |
| `--diagnosticProfile <tsc\|native>` | Diagnostic profile |
| `--maxDiagnostics <N>` | Cap reported diagnostics (must be > 0) |
| `--jobs <auto\|N>` | Worker threads (project mode only) |
| `--stubExternalModules` | Suppress non-relative missing-module diagnostics |
| `--no_implicit_any` | Enable `noImplicitAny` |
| `--noLib` | Disable default libs (no standard/DOM globals) |
| `--showSpans` | Debug: force the custom span renderer |
| `--physicalLibs` | Load the standard library from the project's installed TypeScript instead of the bundled snapshot |
| `--typescript-lib-path <DIR>` | Load the standard library from an explicit `lib*.d.ts` directory; wins over `--physicalLibs` |

The three reporting flags never change the diagnostics output: stdout stays
byte-identical to a run without them. `--extendedDiagnostics` and
`--memoryReport` write human-readable blocks to stderr; `--reportJson` writes
to the given file. All three require `--project` and reject `--showConfig`
(no check runs under `--showConfig`, so there is nothing to report). They
compose with `--compatReport`, `--format json`, `--jobs`, and
`--maxDiagnostics` (the report counts all diagnostics, not the truncated
display). All measurements are taken at existing phase boundaries after
checking completes — the flags add no hot-path instrumentation, and a run
without them collects nothing.

### 1.4 Machine-readable run report (`--reportJson <PATH>`)

One JSON object, pretty-printed, with a fixed key order (`schemaVersion`
first; within each object the order below). Metrics the platform cannot
provide are `null` — never omitted, never fabricated.

```json
{
  "schemaVersion": 1,
  "summary": {
    "files": 77,
    "sourceFiles": 1,
    "dependencyDeclarationFiles": 1,
    "defaultLibFiles": 75,
    "diagnostics": 1,
    "wallTimeMs": 156.346,
    "jobs": "auto",
    "allocator": "system"
  },
  "phases": {
    "configProjectLoadingMs": 1.173,
    "fileDiscoveryMs": 0.065,
    "defaultLibLoadingMs": 3.922,
    "packageDeclarationDiscoveryMs": 0.569,
    "importGraphExpansionMs": 0.078,
    "pathMappingResolutionMs": 0.003,
    "checkingMs": 150.008,
    "diagnosticRenderingMs": 0.202,
    "totalMs": 156.346
  },
  "memory": {
    "peakPhysicalBytes": 45089248,
    "finishPhysicalBytes": 45089248,
    "peakRssBytes": 54886400
  }
}
```

- `schemaVersion` — `1`. Bumped on any breaking change to this shape.
- `summary.files` — every file in the checked program;
  `sourceFiles` + `dependencyDeclarationFiles` + `defaultLibFiles` always
  equals `files`. Source files are project-owned files (including root
  declaration files); dependency declaration files are `.d.ts`/`.d.mts`/
  `.d.cts` under `node_modules/`; default lib files are the physical
  TypeScript `lib*.d.ts` set or the generated fallback subset.
- `summary.diagnostics` — total diagnostics emitted (not capped by
  `--maxDiagnostics`).
- `summary.wallTimeMs` and every `phases.*Ms` value — fractional
  milliseconds (microsecond precision). The phase set mirrors the
  `--timings` categories; `totalMs` duplicates `wallTimeMs`.
- `summary.jobs` — the string `"auto"` when worker selection is automatic
  (the default), otherwise the requested worker count as a number.
- `summary.allocator` — the compiled-in global allocator: `"system"`,
  `"mimalloc"`, `"jemalloc"`, or `"snmalloc"`.
- `memory.peakPhysicalBytes` / `memory.finishPhysicalBytes` — the
  `phys_footprint` peak and at-report values (macOS only; the
  Activity-Monitor-comparable figure). `null` elsewhere.
- `memory.peakRssBytes` — the OS-tracked resident-set high-water mark
  (macOS and Linux). `null` elsewhere.

### 1.5 Compiler API (experimental, not part of this contract)

`surge_ts::api` and the `surge-ts` npm package (`import ts from "surge-ts"`)
expose a read-only subset of TypeScript's compiler API: programs, source
files and syntax trees, and a type checker. The API is experimental and not
covered by this document's guarantees; its method-by-method compatibility is
in [docs/COMPILER_API.md](docs/COMPILER_API.md).

---

## 2. Verified-perfect features (exact `tsc` parity)

"Perfect" here means **exact parity with the upstream TypeScript compiler under
a regression gate** — diagnostic code-count and file/code/line match — on the
fixtures and projects listed below. These are guarded by the oracle harness and
cargo fixtures, so a regression fails CI.

### 2.1 Real projects held at exact parity

| Project | Shape | Gate |
| --- | --- | --- |
| **ky** | [sindresorhus/ky](https://github.com/sindresorhus/ky) 2.0.2 Fetch-API/DOM (`exactOptionalPropertyTypes`, ~29 files) | `pnpm run real:ky:test` — strict false-positive corpus (`tsc = 0`) |

A strict false-positive corpus is one where `tsc` reports zero diagnostics, so
*any* surge diagnostic is a regression rather than a judgement call.

`unnamed` (a local Next.js App Router app: `moduleResolution: bundler`, `jsx:
react-jsx`, `strict`, `paths`) is also a `tsc = 0` corpus but is **not** held at
exact parity: `pnpm run real:unnamed:test` asserts a count ceiling on surge's
over-reports, a ratchet that only moves down. Its current count is in
[`CURRENT_STATUS.md`](CURRENT_STATUS.md#real-project-compatibility).

Both gates **skip** cleanly when the project or the `typescript` package is absent (the
source is never vendored) — a skipped gate is not a passing gate. Which of
these was last actually measured, and with what result, is recorded in
[`CURRENT_STATUS.md`](CURRENT_STATUS.md#real-project-compatibility).

`ofetch` is additionally held at exact parity on a non-zero baseline (`tsc`
reports one `TS5108`, matched by surge at the same file/code/line and message
text).

> Note: ky's source-level parity is exact, but three non-zero suppression
> counters (`suppressedRustOnly`, `suppressedDeclaration`, `externalModuleStubs`)
> sit behind that number; they were audited on 2026-06-20 (the audit document was
> removed on 2026-09-27 and git history keeps it).
> The source-file comparison itself is exact.

### 2.2 Oracle-gated preset registry

Every fixture registered in `scripts/oracle/compare-tsc.ts` is held at the
normal gate (code-count and file/code/line). The registry grows as fixtures are
added; the current count and its last verified result are in
[`CURRENT_STATUS.md`](CURRENT_STATUS.md#gates). The `diagnostics-pack` preset is
held at exact **31/31** emitted-diagnostic parity (duplicate-declaration
TS2451/TS2393, TDZ TS2448+TS2454, missing-return TS2355/TS2366 span placement,
use-site generic-arity TS2314/TS2315).

Run the gate:

```bash
pnpm run oracle:test                       # gated preset comparison
pnpm run oracle:sweep -- --all --maxDiagnostics 200
```

The verified feature areas (each backed by one or more gated presets):

**Modules & resolution**

- Relative imports: deep paths, directory `index`, `.js`→`.ts` extension
  substitution, generated-relative graphs.
- Bare package imports (scoped/unscoped) and exact subpaths → declaration
  entrypoints (`types`, `typings`, `exports[...].types`, `index.d.ts` fallback).
- Package `exports` (conditional, pattern, subpath, custom condition,
  `export =`, `export *`), `imports` field, package self-name, `typesVersions`.
- `paths` aliases and wildcard `paths` import graphs.
- Ambient `declare module` blocks, ambient-module reopen/merge, side-effect
  imports (TS2882), `--stubExternalModules` suppression, unresolved → TS2307.

**Declarations & merging**

- `interface` merging (same file, across files, conflict → TS2717), method
  merge, `declare global` interfaces/`Window`.
- Module augmentation (add export, package interface, unresolved-no-cascade).
- `class` / `declare class`: instance/static members, accessors, constructor
  arity, `extends` heritage (incl. DOM `Request` base), rest params, `this`
  body, `typeof` static side, class/interface merge policy.

**Types & inference**

- Generics: explicit type-argument instantiation, narrow call-site inference
  (direct, repeated-param, array-element), return substitution, type-parameter
  scope.
- Indexed access (`T[K]`, `T[keyof T]`, tuple numeric), mapped types
  (`{ [K in keyof T]: T[K] }`), type operators (`typeof`, `keyof`).
- Conditional types (incl. distributive) and template-literal types (keyof,
  number, union expansion, generic substitution) on the supported subset.
- Utility types: `Record`, `Partial`, `Pick`, `Omit`, `Required`, `Readonly`,
  `Exclude`, `Extract`, `NonNullable`, `ReturnType`, `Parameters` (narrow).
- Intersections, type assertions (`as`), `satisfies`, non-null `!`,
  optional chaining `?.` + `??`, `as const`.

**Flow & functions**

- Use-before-declaration / TDZ, unassigned-local flow, truthy / `typeof` /
  `instanceof` / `Array.isArray` / discriminant narrowing.
- Function rest params, default/optional params, binding-pattern params,
  destructuring locals, callback parameter scope, contextual callback object
  properties, async body locals & return flow.

**Default libs & globals**

- Physical `lib*.d.ts` graph loaded by default (ES + DOM): `Array`, `Map`/`Set`,
  `Promise` (incl. `new Promise` executor), `URL`, `Event`, `FormData`,
  `HTMLElement`, index signatures, `for…of` over iterators.
- `--noLib` correctly removes standard/DOM globals; `lib` option selection.
- `@types` discovery via configured `types` / `typeRoots`, `/// <reference
  types>`, `node:*` protocol imports (with/without `@types/node`).

**JSX / TSX**

- `.tsx` parsing, JSX elements/fragments/attributes/children, function-component
  props, intrinsic elements, imported-component props, member tags,
  DOM-physical-lib props, generic-angle disambiguation, unresolved-no-cascade.

> **Scope boundary:** JSX support is element/prop validation, **not** the JSX
> transform or generated Next.js route types. The current gap list is in
> [`CURRENT_STATUS.md`](CURRENT_STATUS.md#known-limitations).

---

## 3. How to re-verify

```bash
pnpm test
```

```bash
cargo test --workspace
```

```bash
pnpm run oracle:test
```

```bash
pnpm run oracle:sweep -- --all --maxDiagnostics 200
```

```bash
pnpm run real:ky:test
```

```bash
pnpm run real:unnamed:test
```

A target fails the gate only on diagnostic code-count or file/code/line
mismatch; message-text and span/column drift are reported but non-gating unless
`--strictMessages` / `--strictSpans` is passed. The real-project gates *skip*
when the project or the `typescript` package is absent — check the output, a
skip is not a pass.

The last recorded results for all of these are in
[`CURRENT_STATUS.md`](CURRENT_STATUS.md#verification-snapshot).
