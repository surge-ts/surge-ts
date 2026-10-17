# Performance Invariants

Rules that keep `surge-ts` checking fast, memory-bounded, and deterministic.
Each rule is enforced by (or distilled from) code in this workspace; the cited
files are the source of truth. Violations of these rules have historically
produced multi-gigabyte RSS regressions or diagnostic nondeterminism, so
changes in these areas require an interleaved before/after benchmark and an
oracle sweep (see the "Performance and correctness guardrails" section of
[AGENTS.md](../AGENTS.md)).

These rules are current and enforceable. The measured *results* of past
optimization work are point-in-time records and live in
[docs/perf/](perf/); the current measured state is
[CURRENT_STATUS.md](../CURRENT_STATUS.md).

## Checker-context rules

- **Never deep-clone `CheckerOptions`.** `CheckerContext.options` is an
  `Arc<CheckerOptions>` (`crates/surge-ts-checker/src/context/options.rs`). The options
  carry the project-wide module-resolution tables
  (`resolved_modules` / `resolved_modules_by_importer`), so a per-module deep
  clone copies the whole resolution map. Shadow contexts must be built through
  `CheckerContext::new_with_shared_options`, which shares the existing handle.
- **Never retain a broad `CheckerContext` in program-lifetime values.**
  Program-lifetime structures (caches, export tables, declaration payloads)
  must not capture a whole context. Resolution state that must survive a
  context is captured as an interned `DeclarationEnvironmentHandle`
  (`DeclarationEnvironmentStore` in `context/declaration_environment.rs`), which stores only the
  environment-relevant fields and is torn down with the other program caches.
- **Options are shared and immutable for the run.** Every context clone in a
  run reads the same `Arc<CheckerOptions>`; nothing mutates options after
  program start.
- **Reset per-file transient state at the file boundary.** Both serial and
  parallel checking reuse one `CheckerContext` per worker (serial is a single
  worker; see `check_program_files_serial` in
  `crates/surge-ts-checker/src/program/check_files.rs`).
  `CheckerContext::begin_file_check` is the file-region reset: it swaps in a
  fresh `resolved_named_types` map (swapped, not cleared in place — retained
  snapshots may still hold the old `Arc`), clears the diagnostic dedup index,
  and clears the per-file utility-key overlay. Any new per-file state must be
  added to that reset, or it silently accumulates across files on a reused
  worker context.
- **Preserve lexical declaration environments.** A declaration's body resolves
  in its declaring module's scope, never the consumer's:
  `module_scope_by_file` / `module_local_values_by_file` are the authoritative
  per-file fallbacks, and `lookup_ignores_local_table` (`context/mod.rs`) prevents
  a consumer-local type name from shadowing a dependency's own lexical scope
  while its body is being expanded cross-file. Do not "simplify" resolution to
  consult the consumer's local table first — it is wrong per tsc and makes
  expansions depend on which module triggered them, defeating cross-module
  expansion reuse.

## Hashing rules

- Workspace FxHash (`surge_ts_types::fx::FxHashMap` /
  `FxHasher`) is allowed for **trusted internal keys**: source file paths,
  declaration names, and structural type fingerprints. The threat-model note
  at the top of `crates/surge-ts-types/src/fx.rs` is the justification — read
  it before reusing the type elsewhere.
- Identity hashing (`PrehashedU64Map`) is only for keys that are already
  high-entropy 64-bit fingerprints; never feed it raw or attacker-influenced
  values.
- Structures whose iteration order is observable must preserve insertion
  order. `PropertyMap` is an `IndexMap` because tsc's diagnostic rendering
  depends on declaration order (`crates/surge-ts-types/src/object.rs`); its
  equality is order-independent, so any hash derived from it must combine
  order-independent fields commutatively (see the property-name combiner in
  `dedup_key_into`, `crates/surge-ts-types/src/union.rs`).
- Diagnostic and output determinism must never depend on hash-map iteration
  order: check results merge by file index, and CLI report tables are
  explicitly sorted (`crates/surge-ts-cli/src/report.rs`). Any new output that
  reads a map must sort before rendering.
- **Non-randomized hashing on external/untrusted input requires an explicit
  threat-model review.** FxHash is not always safe. The existing maps are safe
  because their keys are project file paths, declaration names, and interner
  fingerprints; a new map keyed by content an attacker can shape freely must
  justify its hasher choice or use the default randomized hasher.

## Cache-safety rules

- **No environment-insensitive cross-module caches.** `resolved_named_types`
  is per-file by design: a resolution depends on the consumer file's
  environment. Program-wide caches must key on everything the result depends
  on — declaration identity, resolved arguments, and environment identity
  (cf. `InterfaceInstantiationKey` carrying `InterfaceEnvironmentIdentity`,
  and the declaration-environment discriminator that participates in program
  canonicalization; `crates/surge-ts-checker/src/context/interface_keys.rs`).
- **No preliminary/final pass collision.** Preliminary module-analysis
  results are superseded by the final round; they must not install first-wins
  global state. `declare global` augmentation *values* are lowered only in the
  final analysis round (`lower_global_augmentation_values` flag in
  `crates/surge-ts-checker/src/program/binding.rs`), because insertion into
  `ambient_global_symbols` is first-wins — a value typed against the
  incomplete preliminary environment would permanently shadow the correctly
  typed final-round value.
- **Never cache degraded or diagnostic-producing results program-wide.**
  `had_error` expansions are not interned
  (`crates/surge-ts-checker/src/infer/types/cache/mod.rs`); a degraded result
  cached program-wide would freeze one file's failure into every consumer.
- **Never cache fallback `Unknown`.** `Type::Unknown` is the
  graceful-degradation sentinel, not a real type: canonical-store
  fingerprinting refuses it (the constructor falls back to an uninterned
  payload; `crates/surge-ts-types/src/store.rs`), and instantiation interning
  skips `is_unknown()` results.
- **Never publish recursion-in-progress results.** The
  `DeclarationResolutionState::Resolving` marker yields an uncached `Unknown`
  to break cycles; only completed resolutions are memoized, and generic
  instantiations are cached only when independent of the enclosing resolution
  context (`resolving` stack empty at the frame, no cycle re-entering below
  the frame floor — `lowest_cycle_target_index` in `context/mod.rs`).
- **Preserve exact overload order and duplicates.** Overload group templates
  keep declaration order and duplicate signatures
  (`InterfaceMethodOverloadGroupTemplate.ordered_members`; pinned by
  `physical_lib_overload_cache_preserves_declaration_order_and_duplicates` in
  `crates/surge-ts-checker/src/infer/types/cache/physical_interface.rs`). Deduplicating or
  reordering overloads changes call resolution.
- Bounded program caches use per-declaration bucket caps
  (`GENERIC_INSTANTIATION_BUCKET_CAP`, currently 4096); over-cap entries are
  simply recomputed, so any cap yields identical diagnostics — keep that
  property when adding a bounded cache.
- Every program cache joins the end-of-run teardown
  (`CheckerContext::clear_program_type_caches` plus `ProgramTypeStore::clear`
  in `program/mod.rs`), which breaks the snapshot/`Arc` cycles that otherwise
  leak each run's type graph. A new program-lifetime cache must be added to
  that teardown.

## Prohibited patterns

```
(*ctx.options).clone() in per-module paths
full CheckerContext clone per file
persistent Vec<Type> where a canonical list ID exists
Arc::new persistent type payload before interner lookup
getenv in hot loops
default HashMap in trusted ID-heavy hot paths without justification
repeated String cloning for canonical paths
consumer-local lookup before dependency lexical scope
```

Notes on individual patterns:

- Environment gates are read once per process via `OnceLock`
  (`canonical_store_enabled` and friends in
  `crates/surge-ts-types/src/store.rs`; the bucket-cap override in
  `infer/types/cache/mod.rs`). A `std::env::var` call inside a per-type or
  per-lookup path reintroduces a syscall into loops measured at millions of
  iterations per run.
- Canonical paths are shared as `Arc<str>` through
  `canonicalize_if_exists_arc` (`crates/surge-ts-checker/src/paths.rs`); a
  cache hit is a refcount bump, not a fresh `String`.
- Type payloads go through the interner first (`FunctionType::new`,
  `UnionType::new`, `ObjectType::new`); `Arc::new` on a fresh payload is the
  miss/fallback path only.

## Retained-memory model (implemented)

The retained-memory program landed as the series ending at `8f0c3a9`; the
region model, its reset/drop boundaries, and the mechanisms later work must
preserve are documented in
[crates/surge-ts-checker/MEMORY_REGIONS.md](../crates/surge-ts-checker/MEMORY_REGIONS.md),
with the measurement evidence and the rejected designs in
[MEMORY-OPTIMIZATION-REPORT.md](perf/MEMORY-OPTIMIZATION-REPORT.md). The
memory-lifetime rules that changes in this area must obey are listed in
[AGENTS.md](../AGENTS.md) § "Memory-Lifetime Rules".

Designs that were evaluated and **deliberately not implemented** (a pooled
`WorkerScratch`, in-place `resolved_named_types` reuse, per-file
`lazy_resolution_snapshot` clearing, borrowed statement checking) are listed
with their reasoning in `MEMORY_REGIONS.md` § "Evaluated and deliberately not
implemented". Do not re-propose them without new measured evidence.

> An earlier revision of this section said the retained-memory work was
> "being explored on a separate branch" with "no current code implementing
> it". That is no longer true.
