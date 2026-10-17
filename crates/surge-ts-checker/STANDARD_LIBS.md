# The Bundled TypeScript Standard Library

surge ships its own version-pinned copy of TypeScript's `lib.*.d.ts`
declarations. A released binary checks standard built-ins without `typescript`,
`node`, `npm`, or `node_modules` being present on the machine or in the checked
project.

The bundled version is recorded in
[`generated-libs/manifest.json`](generated-libs/manifest.json) and is available
at runtime as `surge_ts_checker::lowlevel::bundled_typescript_version()`.

## Where the declarations come from

```
generated-libs/lib.*.d.ts      vendored, checked in, byte-identical upstream
        |
        |  build.rs  (include_str!, one entry per file)
        v
EMBEDDED_LIBS table             read-only data inside the binary
        |
        |  EmbeddedLibSource
        v
reference-graph loader  ->  parser  ->  ambient-global pipeline
```

Nothing reads `generated-libs/` at runtime. The directory is a build input only,
so moving or deleting the source tree after building does not change what the
binary checks against.

## Source selection

`LibSource` abstracts where declaration text comes from. Two implementations
exist, and the graph walk, name normalization, and parse path are shared:

| Implementation | Used for |
| --- | --- |
| `EmbeddedLibSource` | the bundled snapshot (default) |
| `DirectoryLibSource` | an explicit on-disk `lib/` directory |

Precedence is explicit, and an installed `typescript` package is **never**
picked up implicitly:

```
--typescript-lib-path <dir>        explicit directory
        v
--physicalLibs                     discover the project's installed TypeScript
        v
bundled snapshot                   default
```

`--physicalLibs` also responds to a `.physicalLibs` marker file beside the
resolved `tsconfig.json` and to `SURGE_PHYSICAL_LIBS`. When an override is
requested but cannot be honoured, surge warns and uses the bundled snapshot
rather than checking against no standard library.

## Virtual paths

Embedded files are addressed as `<surge-lib>/lib.<name>.d.ts`. The angle
brackets cannot occur in a resolved project path, so these identities never
collide with user files, and a diagnostic pointing at one is visibly from the
bundled snapshot rather than from somewhere on disk. The identity is stable
across runs and machines, which is what makes it usable as a cache key.

## Lib selection

1. **`noLib: true`** loads nothing, from any source.
2. **`compilerOptions.lib`** entries are normalized the way TypeScript
   normalizes them — trimmed, lowercased, with optional `lib.` / `.d.ts`
   affixes — so `ES2022`, `es2022`, and `lib.es2022.d.ts` all select
   `lib.es2022.d.ts`. An entry with no matching file is reported as a warning.
3. **`target`** supplies the seed when `lib` is absent, matching how `tsc`
   derives the default lib:

   | target | seed |
   | --- | --- |
   | `ES5` (and `ES3`) | `lib.es5.d.ts` |
   | `ES2015` / `ES6` | `lib.es6.d.ts` |
   | `ES2016` … `ES2025` | `lib.<target>.full.d.ts` |
   | `ESNext` | `lib.esnext.full.d.ts` |

   Pre-ES2016 targets have no `.full` aggregate; upstream names the ES2015 one
   `lib.es6.d.ts`. A unit test asserts every target maps to a seed the snapshot
   actually contains, because a missing seed silently yields no standard library.
4. **`/// <reference lib="..." />`** directives are followed recursively from
   each seed, deduped by source identity, cycle-guarded, and emitted
   dependency-first. No dependency list is hardcoded: the graph is whatever the
   vendored files declare.

## Updating the snapshot

See [scripts/lib/README.md](../../scripts/lib/README.md). Normal `cargo build`
never downloads anything; refreshing the vendored files is an explicit
maintainer step.

## Licensing

The vendored declarations are TypeScript's, redistributed under Apache-2.0. Each
file keeps its upstream copyright header, and the upstream `LICENSE.txt` and
`NOTICE.txt` are vendored alongside them in `generated-libs/`. The vendor script
refuses to run if either is missing.

## Known gaps

Full DOM coverage beyond everyday types is not guaranteed; the checker-surface
gaps that apply to lib APIs are tracked in
[CURRENT_STATUS.md § Known limitations](../../CURRENT_STATUS.md#known-limitations).

The three gaps this section used to list — overloaded lib APIs answered from one
signature, plain `TS2304`/`TS2339` where `tsc` reports `TS2550`/`TS2552`, and an
implicitly-`any` `addEventListener` callback parameter — were refuted on
2026-10-17 with a reduced probe at `d0f89bae` (release binary built from a
working tree carrying uncommitted checker edits): surge's output matched
`tsc` 7.0.2 on all three.
