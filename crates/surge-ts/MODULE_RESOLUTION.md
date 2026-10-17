# Module Resolution

Since `a46eb7ea` (2026-10-13) module resolution is a port of typescript-go's
resolver (`internal/module/resolver.go`), and program file discovery follows
tsc's `processImportedModules`. Behaviour questions are answered by the tsgo
source, not by this file; this file only says where the port lives and how to
compare it against `tsc`.

The pre-port audit this file used to carry (the loader's resolver call-site
inventory, extension matrix, and deferred-feature list) described the legacy
path and was removed on 2026-10-17; `git show e5fc8b09:crates/surge-ts/MODULE_RESOLUTION.md`
has it. Its "not supported" entries were not re-verified against the port, so
do not cite them.

## Layout

| Path | Port of | Role |
| --- | --- | --- |
| [src/resolver/mod.rs](src/resolver/mod.rs) | `internal/module/resolver.go` | `ModuleResolver`: module names and type reference directives under `node16`, `nodenext` and `bundler` (the only kinds tsgo has) |
| [src/resolver/config.rs](src/resolver/config.rs) | compiler options → resolver options | The resolver's options from a loaded config, with tsgo's defaults (`paths`, `rootDirs`, `typeRoots`, `preserveSymlinks`, `moduleSuffixes`, conditions, …) |
| [src/resolver/packagejson.rs](src/resolver/packagejson.rs) | `internal/packagejson` | The `package.json` fields the resolver reads, validated as tsgo validates them |
| [src/resolver/tspath.rs](src/resolver/tspath.rs) | `internal/tspath` | Slash-normalized path strings and extension constants |
| [src/resolver/host.rs](src/resolver/host.rs) | resolution host | The resolver's view of the disk through the loader's memoized probes |
| [src/resolver/validate.rs](src/resolver/validate.rs) | — | Replays a `tsc --traceResolution` log through the resolver (see below) |
| [src/module_discovery.rs](src/module_discovery.rs) | `processImportedModules` | Resolves each loaded file's module names, records every answer for the checker, and loads and scans the files the program takes in (`allowJs`, `maxNodeModuleJsDepth`, `resolveJsonModule`, `allowArbitraryExtensions` gates) |

The checker never reads new files: it binds imports and exports against the
resolutions `module_discovery` recorded.

## Comparing against `tsc`

- `SURGE_RESOLVER_VALIDATE=<trace>` replays the resolutions a
  `tsc --traceResolution` log records through `ModuleResolver` and reports each
  answer that differs (`RESOLVE-DIFF`), then a summary.
- `SURGE_LEGACY_RESOLUTION=1` restores the loader's own package and import-graph
  passes (`import_graph.rs`, `package_declarations/`, `path_mapping.rs`) in
  place of the port, for comparison only. It is not a supported mode.
- Behaviour tests: [tests/cli/module-resolution.test.ts](../../tests/cli/module-resolution.test.ts).
