# Architecture

`surge-ts` is a Rust-based TypeScript noEmit compatibility checker: it aims for
tsc-compatible diagnostics in noEmit-style project checks. `TypeScript` below
refers to the language/ecosystem being checked, not the project.

This document describes the **design**. For the current measured state — gate
results, real-project parity, known limitations, feature gates, benchmarks —
see [CURRENT_STATUS.md](CURRENT_STATUS.md).

This workspace is organized as small crates with stable public façades and internal modules that can evolve without forcing broad API churn.

> Naming note: the public project and reports are named `surge-ts`, and the CLI
> binary command is `surge`. The internal Cargo crates are named `surge-ts-*`
> (e.g. `surge-ts-cli`, `surge-ts-checker`) and crate directories live under
> `crates/surge-ts-*`. Custom diagnostic codes use the `surge::` namespace (e.g.
> `surge::project-has-no-source-files`).

## Current compatibility baseline

> **Numbers live elsewhere.** The verified gate results, real-project parity
> matrix, current limitations, and benchmark figures are in
> [CURRENT_STATUS.md](CURRENT_STATUS.md); the stable/exact-parity contract is in
> [PUBLIC_API.md](PUBLIC_API.md). This section describes the *shape* of the
> baseline, not its current counts.

Project mode loads the physical `lib*.d.ts` graph from the pinned local
`typescript` package by default. Standard/DOM/global library surfaces and the
utility-type ecosystem come from those loaded `.d.ts` declarations, not from
Rust-side synthetic globals. The generated default-lib subset is only a fallback
used when the `typescript` package cannot be found (and the single-file support
path), not the normal correctness source of truth. `noLib: true` keeps the
standard/DOM globals unavailable. The internal type IR still models primitives
and object/function/union/array/tuple shapes, but those are language-level type
representations rather than ambient library declarations. Node core-module
knowledge, where it exists, is diagnostic/resolution support (e.g. missing-module
hints), not Node global type synthesis.

The oracle gate is **diagnostic code-count and file/code/line**; message text
and span/column are separate, non-gating dimensions unless `--strictMessages` /
`--strictSpans` are passed. Nothing about this baseline is a claim of full
TypeScript, `lib.d.ts`, DOM, Node, or React parity.

## Historical version notes

The version-tagged milestone notes (`v0.4x`–`v1.2.5`) that used to sit here were
removed on 2026-09-27; git history keeps them. They described superseded
states — the "synthetic built-ins" and "generated default-lib" models among
them — and the `v0.x` / `v1.x` labels are internal milestone markers, not
releases or crate versions
([CURRENT_STATUS.md § Versioning](CURRENT_STATUS.md#versioning)).

## Crate layout

| Crate | Responsibility |
| --- | --- |
| `surge-ts-syntax` | Parse TypeScript source into a simplified AST |
| `surge-ts-types` | Core type representation, display, unions, assignability, and the canonical program type store |
| `surge-ts-checker` | Semantic checking and diagnostic emission |
| `surge-ts-diagnostics` | Diagnostic codes, catalog, generated accessors, and rendering |
| `surge-ts-config` | `tsconfig.json` loading, normalization, and file discovery |
| `surge-ts-tsc-syntax` | Port of tsgo's scanner, parser and binder: tsc-exact syntactic diagnostics, and the syntax tree and symbols of the compiler API |
| `surge-ts` | Embeddable umbrella crate: `Project` (config load, package/`paths` resolution, default-lib loading, import-graph expansion) plus re-exported checker APIs, and the compiler API (`surge_ts::api`) |
| `surge-ts-cli` | CLI orchestration (built on `surge-ts`) |
| `surge-ts-node` | Node addon (napi-rs) behind the `surge-ts` npm package (`packages/surge-ts`), the compiler API from JavaScript |

A second table, mapping each crate to its role plus the codegen crate, is in
[§ Naming](#naming).

## Checking pipeline

Project checking runs the following phases in order (loader phases live in
`crates/surge-ts/src/lib.rs`; checking phases in
`crates/surge-ts-checker/src/program/mod.rs`):

1. **Config load** — `surge-ts-config` loads and normalizes `tsconfig.json`
   (extends chains, include/exclude, compiler options).
2. **File discovery** — the config's include roots are expanded into the
   initial source-file set.
3. **Import-graph expansion** — `Project::check` runs a fixpoint that
   combines `crates/surge-ts/src/import_graph.rs` (relative files,
   `baseUrl`/`paths` mappings) with the package-declaration resolvers
   (`package_declarations.rs` / `package_resolution.rs`: package entrypoints,
   `types`/`typeRoots`, `/// <reference types>` directives) until no new file
   is discovered. The physical `lib*.d.ts` graph from the local `typescript`
   package is loaded here.
4. **Parse** — `parse_program_files` parses all inputs into the simplified
   AST; parsing may fan out to parse workers (one oxc allocator per thread,
   dropped after parsing). Each file is classified by its resolved physical
   path (`classify_file_kind` → `FileKind`: root source/declaration,
   dependency declaration, generated declaration, physical default lib),
   which selects its checking policy (declaration-backed deferral,
   diagnostic suppression, ambient lowering).
5. **Module analysis and binding** — ambient globals/augmentations/ambient
   modules are collected; then a preliminary module-analysis round, a
   multi-round export-table/import-binding/resolution-scope fixpoint, and a
   final module-analysis round produce the shared program state (see
   `crates/surge-ts-checker/PROGRAM_CHECKING.md` for why there are two
   analysis rounds and what may only happen in the final one). Preliminary
   structures are dropped at the `preliminary_release` boundary.
6. **Check** — per-file checking over the shared read-only state, serial or
   parallel (`--jobs`); worker results merge in loaded-file order.
7. **Render** — the CLI groups diagnostics by file in loaded-file order and
   renders via `surge-ts-diagnostics` (`tsc`/`custom`/`json` styles). Report
   tables are explicitly sorted; no output depends on hash-map iteration
   order.

At the end of a run the program caches are torn down
(`clear_program_type_caches` + `ProgramTypeStore::clear`) so a long-lived
embedding process does not retain the run's type graph.

## Canonical type stores

`surge-ts-types` owns a per-run `ProgramTypeStore`
(`crates/surge-ts-types/src/store.rs`) that interns structural type payloads
so identical types are shared instead of re-allocated:

- **Program ownership.** `check_program_with_stats_and_jobs` creates one store
  per run and installs it thread-locally (`with_program_type_store`); each
  check worker installs the same store on its thread. IDs embed a 32-bit
  owner tag, so a `TypeListId`/`FunctionTypeId`/`UnionTypeId`/`PropertyMapId`
  from one program can never be dereferenced against another program's store
  (`belongs_to`). **Type IDs must never cross program owners** — an ID is only
  meaningful together with the store that minted it.
- **Immutable canonical payloads.** Interned payloads
  (`Arc<FunctionTypePayload>`, `Arc<UnionTypePayload>`, `Arc<PropertyMap>`,
  `Arc<[Type]>` parameter lists) are write-once; consumers share them by
  handle. Pointer equality of a shared payload short-circuits structural
  comparison on hot paths.
- **What is canonicalized.** Parameter type lists, function payloads, union
  member lists, and object property maps, plus an overload-merge pair cache.
  Lookups hash a bounded structural fingerprint and then confirm by exact
  structural equality inside the bucket, so a fingerprint collision can never
  return a wrong type.
- **Canonical vs fallback payloads.** Interning is best-effort: values whose
  fingerprint is refused — `Type::Unknown` (the degradation sentinel),
  references that retain resolution context, or over-budget/deep structures —
  fall back to an ordinary uninterned `Arc` payload with no ID. Fallbacks are
  semantically identical, just unshared.
- **Concurrency.** The store is sharded (64 shards per table) behind mutexes
  with a contention counter; it is shared across check workers via `Arc`.
- **Cleanup boundary.** `ProgramTypeStore::clear` at end of run drops every
  interned payload still uniquely owned by the store; the checker-side caches
  that reference them are cleared first (`clear_program_type_caches`).

Per-type details: [FUNCTION_TYPES.md](crates/surge-ts-types/FUNCTION_TYPES.md),
[UNION_TYPES.md](crates/surge-ts-types/UNION_TYPES.md). Memory-region and
lifetime rules: [MEMORY_REGIONS.md](crates/surge-ts-checker/MEMORY_REGIONS.md).
Cross-cutting performance rules:
[docs/PERFORMANCE_INVARIANTS.md](docs/PERFORMANCE_INVARIANTS.md).

## Boundary Rules

- `lib.rs` in each crate should stay façade-like.
- New feature work should land in focused modules, not in crate root files.
- Public crate-root exports should stay stable unless a breaking change is intentional.
- Internal helpers should prefer `pub(crate)` visibility.
- Checker inference is split into expression inference and parsed-type
  resolution; keep those two responsibilities separate.
- New feature work should add focused modules rather than re-expanding
  monolithic files. Config, syntax, and checker logic stay in their dedicated
  submodule trees rather than returning to crate-root files.
- The next phase should be chosen from measured output — the oracle sweep and
  `--compatReport` — not from a fixed feature wish list.

  > Historical note: earlier revisions of this section described interfaces,
  > arrays/tuples, literal types, and imports/exports as *future* work with
  > deliberately minimal surfaces. All of those have landed and are pinned by
  > oracle presets; see [PUBLIC_API.md](PUBLIC_API.md) § 2 for the verified
  > feature areas. The "Suggested Homes For Future Features" table below is
  > kept as a routing guide for where such work belongs, not as a list of what
  > is missing.

## Suggested Homes For Future Features

- Interface parsing and checking: `surge-ts-syntax` and `surge-ts-checker`
- Arrays and tuples: `surge-ts-syntax`, `surge-ts-types`, and `surge-ts-checker`
- Literal types: `surge-ts-syntax`, `surge-ts-types`, and `surge-ts-checker`
- Imports and exports: `surge-ts-syntax`, `surge-ts-checker`, and `surge-ts-config`
- Program checking: `surge-ts-checker` and CLI project mode
- Compatibility reporting and triage: `surge-ts-cli` and `surge-ts-checker`
- Oracle comparison: `scripts/oracle/compare-tsc.ts` for project and file mode
  validation (including --ignoreConfig for standalone file checking) plus diagnostic drift measurement
- New diagnostics: `surge-ts-diagnostics` (catalog-driven, including CLI-only diagnostics like TS5112)
- New project-visibility diagnostics: `surge-ts-cli` may emit a custom `surge::project-has-no-source-files` diagnostic when project discovery returns zero source files
- Checker-local path normalization lives in `surge-ts-checker`; config loading and normalization remain in `surge-ts-config` for tsconfig discovery.
- Default-lib loading lives in `surge-ts-checker` and is shared by single-file and program checking. The declarations are a version-pinned TypeScript snapshot embedded in the binary at build time and served through the `LibSource` abstraction, so no `typescript` package is needed at runtime; an on-disk lib directory is used only when explicitly selected. Lib selection from tsconfig feeds the real ambient declarations either way. See [crates/surge-ts-checker/STANDARD_LIBS.md](crates/surge-ts-checker/STANDARD_LIBS.md).

## Memory-lifetime model

Retained memory is governed by ownership lifetimes, not by cache pruning. The
canonical type stores in `surge-ts-types` hold `Weak` payload references with
monotonic never-reused IDs; `CheckerArena` registers destructors for every
`Drop`-requiring payload; declaration environments capture compact
stamp-deduplicated table snapshots instead of table copies; qualified-import
payloads are shared across importers while explicitly retaining their owning
arena; and superseded analysis, AST, binding-generation, and TLS state is
released at true-death lifecycle boundaries. The full region model, the
prohibited lifetime shortcuts (expansion-cache pruning, broad re-export
payload sharing, environment-insensitive result sharing), and the measurement
tooling (`SURGE_RETENTION_CENSUS`, `SURGE_PAUSE_AT_STAGE`, `SURGE_RSS`) are
documented in
[crates/surge-ts-checker/MEMORY_REGIONS.md](crates/surge-ts-checker/MEMORY_REGIONS.md)
and [docs/perf/MEMORY-OPTIMIZATION-REPORT.md](docs/perf/MEMORY-OPTIMIZATION-REPORT.md);
the canonical-store retention rules live in
[crates/surge-ts-types/FUNCTION_TYPES.md](crates/surge-ts-types/FUNCTION_TYPES.md).

## Diagnostics

Diagnostics are catalog-driven in `surge-ts-diagnostics`.
The Rust accessors are generated from `diagnostic-messages.json`, and spans remain a checker/parser concern rather than a catalog concern.

The default human-readable CLI output is `tsc`-compatible (`render_diagnostics_tsc`):
`--diagnosticStyle <tsc|custom|json>` selects the renderer and `--pretty` controls
the code-frame form. JSON output is unchanged and still drives the oracle harness,
so this rendering layer never affects diagnostic comparison. See the CLI README for
flag details.

## Declaration Ingestion

The `.d.ts` ingestion surface below is current behavior (it originated in the
`v0.64`/`v0.65` milestones).

- Loaded `.d.ts` files can contribute ambient globals and exact `declare module "pkg"` blocks.
- Ambient modules resolve before package import stubbing fallback.
- Default exports, namespace imports, named re-exports, type-only re-exports, and star re-exports are pinned for the supported ambient-module subset.
- Duplicate `interface` declarations merge (same file, across global files, reopened `declare module` blocks, and `declare global`); a conflicting property type reports TS2717 with the first declaration winning. Duplicate ambient `var`/`const`/`function` globals stay first-wins / pinned.
- A `declare module "pkg"` block in a module file augments an already-resolved target (merging exported interfaces, adding new exported functions/types); augmenting an unresolved target keeps the TS2307 no-cascade policy.
- Unsupported declaration syntax stays parser-safe and emits a stable pinned diagnostic.
- Current project mode supports declaration-side modern package resolution (conditional/pattern `exports`, `imports`, `typesVersions`, self-name), `types`/`typeRoots` and `@types` discovery under TypeScript 6/7 semantics (including the `types: ["*"]` wildcard — see [crates/surge-ts-cli/AUTO_TYPES.md](crates/surge-ts-cli/AUTO_TYPES.md)), recursive `/// <reference types="…" />` following from dependency declaration files, class/static/constructor semantics, a bundled version-pinned `lib*.d.ts` snapshot embedded in the binary, JSX props checking, and a declaration-merging/module-augmentation slice. `baseUrl` non-relative specifier resolution is supported in the loader (the option is deprecated upstream but honored for compatibility). Full `lib.d.ts`/DOM/Node parity, full runtime/JS package resolution, and full TypeScript parity remain out of scope; the current gap list is in [CURRENT_STATUS.md](CURRENT_STATUS.md#known-limitations).

## Naming

The public project and reports are named `surge-ts`; the CLI binary command is
`surge`. The internal Cargo crates are `surge-ts-*` and live under
`crates/surge-ts-*`, with `use surge_ts_*` import paths throughout `src/` and
`tests/`. Custom diagnostic codes use the `surge::` namespace (e.g.
`surge::project-has-no-source-files`), and runtime env vars use the `SURGE_`
prefix (`SURGE_PHYSICAL_LIBS`, `SURGE_TIMINGS`).

| Crate | Role |
| --- | --- |
| `surge-ts` | Embeddable umbrella crate (`Project`, loader phases, re-exported checker APIs) |
| `surge-ts-cli` | CLI orchestration (binary `surge`) |
| `surge-ts-checker` | Semantic checking and diagnostic emission |
| `surge-ts-syntax` | Parsing into the simplified AST |
| `surge-ts-types` | Core type representation and canonical type store |
| `surge-ts-diagnostics` | Diagnostic codes, catalog, generated accessors |
| `surge-ts-config` | `tsconfig.json` loading and discovery |
| `surge-ts-diagnostics-codegen` | Catalog code generation |
| `surge-ts-tsc-syntax` | tsgo scanner, parser and binder port |
| `surge-ts-node` | Node addon of the `surge-ts` npm package |

One internal stats key is intentionally left unchanged: the bench `ts-rust` key
in saved benchmark archive JSON is kept stable so older archives remain
readable.
