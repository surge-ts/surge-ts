# Modules

v0.61 expands the existing relative module-resolution-lite boundary with a
small, pinned module syntax surface for loaded `.ts` files. v0.65 hardens the
ambient-module side of that surface, v0.67 matches TypeScript's TS2882 priority
for unresolved side-effect imports, and v0.84 hardens source/declaration export
visibility for already-loaded modules without adding full package resolution,
`node_modules` runtime semantics, or full TypeScript parity.

## What Is Supported

All relative module-syntax forms are limited to already loaded relative `.ts` files.

- Default imports: `import DefaultThing from "./user";`
- Namespace imports: `import * as user from "./user";`
- Default exports: `export default function ...`, `export default "Ada"`, `export default 123`, `export default true`, and small expression forms the parser already models
- Named re-exports: `export { User } from "./user";`
- Default re-exports: `export { default as DefaultThing } from "./user";`
- Type-only named re-exports: `export type { User } from "./user";`
- Mixed named re-exports: `export { type User, value as renamedValue } from "./user";`
- Star re-exports: `export * from "./user";`
- Namespace re-exports: `export * as userNs from "./user";`
- Named export lists and wrapped declarations from the earlier module-resolution-lite phase
- Side-effect imports and `export {}` module markers

Namespace imports bind a single value symbol whose object type is built from the source module's visible value exports. Default imports bind only a value symbol. Type-only exports stay in the type namespace. Star re-exports forward named value exports and named type exports, but they do not forward default exports. Namespace re-exports bind a conservative namespace object from the target module's visible exports.

v1.2.2 keeps module export lookup read-only by sharing exported symbol handles and caching the namespace object materialization per module, so repeated export resolution no longer deep-clones exported payloads.

v1.2.5 memoizes relative module resolution per run: `resolve_relative_module` caches `(importer, specifier) -> resolved module` in a thread-local map that is cleared at the start of each check (resolved indices are run-specific). This is a pure performance change with no semantic effect — the multi-pass binding/resolution fixpoint resolves the same specifiers several times, so passes after the first reuse the resolved index instead of rebuilding and canonicalizing candidate paths. The resolution order and outcomes below are unchanged.

## Current Policy

- Script files still share top-level `type` aliases, `interface` declarations, and function declarations across files.
- Duplicate `interface` declarations merge across script files and within a single module file; a conflicting property type reports TS2717 and the first declaration's type wins.
- A `declare module "pkg"` block in a module file augments an already-resolved target: its exported interfaces merge and new exported functions/types are added on import. Reopened ambient `declare module "pkg"` blocks in script files merge the same way.
- Augmenting an unresolved module specifier does not make it resolvable; the import still reports TS2307 with no downstream cascade.
- Module files keep their own top-level declarations local to the file.
- Module files do not contribute to the global script namespace.
- Module files do not see script globals under the isolated-module policy.
- Relative resolution only covers already loaded `./`, `../`, `.` and `..` specifiers.
- Missing ordinary relative modules emit TS2307.
- Missing side-effect imports emit TS2882, matching TypeScript's priority for
  `import "pkg";` / `import "./missing";`.
- Missing exported members emit TS2305.
- Named missing exports from the `package-declarations` fixture use TS2614 when TypeScript does.
- Unsupported module syntax stays parser-safe and is pinned with `surge::unsupported-module-syntax`.
- `export * from` follows a pinned conflict policy: local explicit exports win, and the first star export wins when multiple star exports provide the same name.
- Unresolved star re-exports are intentionally kept from cascading extra consumer diagnostics.
- Bare directory specifiers like `.` and `..` can resolve to already loaded `index.*` files in the same directory graph.
- Relative resolution checks the already-loaded graph using the centralized
  candidate matrix in `modules/candidates.rs`: extensionless specifiers probe the
  exact target, `.ts`, `.tsx`, `.d.ts`, then `index.ts`/`index.tsx`/`index.d.ts`;
  the `.mts`/`.cts` flavors are reachable only through explicit `.mjs`/`.cjs`
  specifiers, matching tsc.
- Explicit `.js`/`.jsx` specifiers substitute `.ts`/`.tsx`/`.d.ts` in place;
  `.mjs` → `.mts`/`.d.mts`; `.cjs` → `.cts`/`.d.cts`. A specifier with an
  explicit runtime extension never falls back to a directory-index lookup.

## Non-relative imports and package stubs

v0.70 adds support for package declaration subpath entrypoints. It resolves exact subpath imports (e.g. `pkg/subpath` or `@scope/pkg/subpath`) and exact `exports["."].types` / `exports["./x"].types` declaration entrypoints. Resolved package files act as external modules.

Default mode for unresolved packages:

- reports TS2307 for ordinary non-relative module specifiers
- reports TS2882 for non-relative side-effect imports
- inserts unknown type/value stubs where possible

`--stubExternalModules`:

- suppresses non-relative missing-module diagnostics, including TS2307 and the
  side-effect-import TS2882 form
- keeps unknown stubs
- leaves relative missing modules and resolved package declaration errors unchanged

## Support surface

> Updated 2026-09-01. Several entries in this section previously listed
> already-landed features as unsupported; the corrections are inline below. The
> current gap list is in
> [CURRENT_STATUS.md](../../CURRENT_STATUS.md#known-limitations).

- Modern package declaration resolution is supported on the declaration side: conditional and pattern `exports`, the `imports` field, `typesVersions`, package self-name imports, and exact subpaths. Full runtime/JS entrypoint resolution and `main` parity remain out of scope.
- explicit `paths` aliases and declaration-only package entries share the same internal resolved module map
- `baseUrl` non-relative specifier resolution **is** supported in the loader
  (the option is deprecated upstream but honored for compatibility)
- Project mode loads the physical `lib*.d.ts` graph from the local TypeScript package as ambient default libs by default; the generated default-lib subset is the fallback used only when that package cannot be found. `noLib: true` disables both, keeping standard/DOM globals unavailable. `@types` discovery is supported through configured `compilerOptions.types`/`typeRoots`; full automatic `@types` discovery and full lib.d.ts/Node parity remain out of scope.
- The v0.64/v0.65 declaration-ingestion foundation supports a small loaded `.d.ts` ambient subset, including exact `declare module "pkg"` blocks.
  - Ambient modules and resolved package entrypoints resolve before package stubbing.
  - Default import, namespace import, named re-export, type-only re-export, and star re-export behavior inside exact ambient modules is pinned.
  - Reopened ambient module declarations and module augmentations merge their exported interfaces (declaration merging) within the supported subset; full namespace/enum/overload-ordering merging is out of scope.
  - Exact specifier only.
  - No wildcard ambient module support.
- `import = require(...)` and `export =` **are** supported for the
  declaration-side surface, gated by the `node-decl-callable-namespace-basic`,
  `package-types-export-equals-*`, and `package-types-import-require-*`
  fixtures. Callable-function + namespace merging through `export =` works;
  the *same-file* merge still drops callability (false TS2349).
- Still out of scope: mixed default + named imports in a single clause, full
  CommonJS runtime semantics, and full runtime/JS entrypoint resolution.

## Notes

- Exported generic aliases and interfaces still use the relative module-resolution-lite pass, with explicit type arguments substituted when the imported declaration is instantiated and trailing defaults applied when callers omit type arguments.
- Constraints remain parser-only metadata in this phase.
- Private helper types stay visible through the current module-resolution-lite pass so imported declarations can still resolve them.
- The next phase should continue from compatibility-report output.

## Ambient Modules

Imports try to resolve from ambient external modules defined by loaded `.d.ts` files with `declare module "pkg"` before falling back to package stubbing. Unsupported declaration syntax remains parser-safe and emits the pinned unsupported-declaration diagnostic.
