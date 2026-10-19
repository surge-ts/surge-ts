# Lazy member types and reference-aware relations — design (2026-10-19)

> **Design reference, not a status report.** It describes an intended or
> in-progress design; the numbers it quotes are from the date and commit it
> names. Whether a stage has landed, and under which feature gate, is recorded
> in [CURRENT_STATUS.md](../../CURRENT_STATUS.md).

Status: Stage 0 measured; Stage 1a landed (`934985ba`). The results are under
each stage. Successor to
[MEMBER-LAZY-EXPANSION.md](MEMBER-LAZY-EXPANSION.md), whose Stage 1 was
measured 4x slower on tRPC because relations peel lazy members
([CORPORA-MEMOS-2026-10-18.md](CORPORA-MEMOS-2026-10-18.md)). This design
makes relations reference-aware first, then makes member types lazy.

## Why

Measured on 2026-10-19 at `2b8bee49`, from fast-profile builds (LTO off) with
env-gated probes, on a machine under heavy load from other sessions. Ratios and
counts are the evidence here; wall times are not comparable across runs.

- Eager whole-member expansion of `ZodType<…>` is about 10.1 s of a 22.5 s zod
  run (per-interface inclusive timer, first active frame per name). In trpc the
  zod types its dependencies use take about 20 s of 40 s.
- 62% of `ZodType`/`_ZodType` expansions are the first expansion of their
  (declaration, arguments) key: 11.5k distinct keys over 18.7k memo misses.
  Better memo keys can remove at most about 38% of them. The cost is that every
  instantiation resolves all of its roughly 40 member signatures, and their
  return types, whether or not anything reads them.
- Under `resolve_interface_declaration`, member-annotation resolution
  (`resolve_parsed_type` and named-type resolution) is 58% of the zod check
  thread, 64% of drizzle's and 47% of trpc's. The heritage peel is small
  (zod 5.8%).
- Relations (`is_assignable_to` inclusive) are 5.8% of zod, 9.2% of trpc,
  12.3% of drizzle and 20.5% of tanstack-query.

tsc avoids the fan-out by never expanding members it does not read, and by
relating two references to one generic target through their type arguments.

## How tsc does it

Line numbers are tsgo 7.0.2, `tsc/internal/checker/`.

- **Creating a reference resolves nothing.** `createTypeReferenceEx`
  (checker.go:25015) interns `{target, typeArguments}` per argument list.
  Instantiating a reference maps only its arguments (22133-22139).
- **Members are built on first access, and only read member types are
  instantiated.**
  - `resolveTypeReferenceMembers` (19000) calls `resolveObjectTypeMembers`
    (19011), which builds a member table of instantiated symbols
    (`instantiateSymbol`, 20658). Each symbol's type is computed on demand by
    `getTypeOfInstantiatedSymbol` (16441).
  - Signatures are lazy the same way (`instantiateSignatureEx`, 20524; return
    type at 19906).
  - Index infos are instantiated eagerly (20547).
  - Base types are instantiated with the mapper and their member tables
    resolved, but their property types stay lazy (19032-19059).
- **Same-target references relate by variance.**
  - The branch is `structuredTypeRelatedToWorker` (relater.go:3821). It calls
    `getVariances(target)` and then `relateVariances`, i.e.
    `typeArgumentsRelatedTo` (3903).
  - A pass is final.
  - A failure is final unless a parameter is `Unmeasurable`/`Unreliable` or a
    covariant target argument is `void` (3266-3307). In those cases the
    structural comparison decides, and it can only rescue the pair when the
    variance check did not fail outright (3861-3900).
  - Aliases have the same path through `getAliasVariances` (3389-3407).
- **Variance is measured once per generic symbol** (`getVariancesWorker`,
  1324-1411).
  - The declaration is instantiated with marker super/sub/other types and
    related structurally.
  - The result is cached on the symbol's links.
  - Re-entry during measurement returns an empty slice, which the relation
    treats as `Unknown` (never cached as success).
  - Flags (types.go:300-309): Invariant 0, Covariant 1, Contravariant 2,
    Bivariant 3, Independent 4, plus the bits Unmeasurable 8 and Unreliable 16.

## What surge does today

**Expansion.**
- A user generic interface instantiation is expanded eagerly, then wrapped in
  a `Type::Reference` whose resolver holds the finished expansion
  (`make_type_reference`, `InternedInstantiation`, resolve/named.rs).
- A library interface instantiation is a `LazyInstantiation` reference that
  expands on first peel. The peel then resolves every member.
- Lazy members exist, behind `SURGE_LAZY_IFACE_MEMBERS=1` and only for
  library-scoped files:
  - Property members are deferred by a tier gate.
  - Method members get lazy parameter components when the method is not
    overloaded, and only in the check phase
    (`resolve_function_type_lazy_components`, resolve/structural.rs:718).
  - The return type stays eager; deferring it caused 25 tRPC false positives.
  - The shell drops parameter names, the type-parameter head, the
    polymorphic-`this` flags and the declaration signature.
  - A lazy member reference has no arguments, so it cannot be related by them.

**Relations** (`surge-ts-types/src/assignability/mod.rs`, `subtype.rs`).
- Two references with the same `id` relate by `declared_variances` in
  `assignability_arms` (2369-2426):
  - Passing arguments return `true`.
  - A failure is final only when every parameter is annotated `in`/`out`.
    Otherwise the pair is peeled and compared structurally.
- Before that arm, `type_variable_related` peels both references (1216-1229),
  and the weak-type check peels the target (`has_no_common_properties`, 4748).
  Reaching the variance arm already forces a lazy reference.
- `relation_key` has no `Type::Reference` arm, so (Ref, Ref) verdicts are
  never cached.
- The subtype `Relater` (overload resolution, `remove_subtypes`) has no
  variance path. Its `property_related` peels both sides of every target
  member (subtype.rs:795). This is where `SURGE_LAZY_IFACE_MEMBERS` spent 77%
  of its time. `is_type_identical_to` peels both sides (identity.rs:38).
- `declared_variances` is a syntactic measure (`measure_declared_variances`,
  symbols/type_declarations.rs:632), not tsc's marker measure:
  - A parameter measured covariant stores 0, the same as "unknown".
  - There is no Bivariant or Independent code.
  - The 3-bit encoding has all 8 values in use (reference.rs:363-438).
- Several sites need a concrete `Type::Function` member, so a whole-member
  lazy method reference is not viable:
  - overload folding and `merge_overload_signatures`;
  - the polymorphic-`this` binders;
  - the `infer_property_call` union arm;
  - the method arm of `object_assignability_failure`.

  Laziness goes inside a `Type::Function` shell.

## Measured and rejected (2026-10-19, zod unless noted)

| Idea | Result |
| --- | --- |
| Defer a whole user instantiation whose heritage names a frame on `resolving` | Interface resolutions −32%, but memo stores unchanged; instructions +2%, peak +54 MB, 3 new false positives |
| `SURGE_UPGRADE_ANALYSIS_SCOPES=1` | zod +5.4%, drizzle +35%, trpc +4% instructions; 221 trpc diagnostic lines drift |
| Cache heritage-cycle expansions keyed by the `resolving` suffix | Not a function of the key: 1,558 of 14.7k keys give differing results (`had_error` flips mid-run). Also barred by the rule against caching recursion-in-progress results |
| `SURGE_LAZY_IFACE_MEMBERS=1` (2026-10-18, other session) | trpc 24 s → 104 s, diagnostics drift; 77% of the time is `Relater::object_members_related` peeling lazy members |

## Stages

Each stage is prototyped behind its own env gate in a worktree branch. The
A/B runs one binary with the gate off and on, so build-to-build noise is
excluded. A stage lands only with:
- the 8 corpora byte-identical, or a difference shown to move toward tsc with
  fixtures;
- `oracle:sweep --all` unchanged;
- `--jobs 1` and `--jobs 8` byte-identical on trpc, zod, drizzle-orm and
  tanstack-query;
- an interleaved 3-round A/B of instructions retired and peak footprint on
  those four corpora.

### Stage 0: probes (no behaviour change)

Env-gated counters, printed at program end:
- **P1, relation shapes.** For `is_assignable_to` and the subtype `Relater`:
  - (Ref, Ref) same-id pairs, split by verdict source: argument pass, final
    failure, or structural fallback, plus the fallback's answer;
  - other reference pairs;
  - peels by site (`type_variable_related`, weak-type check, arm peels,
    `property_related`, identity), and how many of them forced an expansion
    rather than a memo hit.
- **P2, member reads.** For property maps built by interface expansion:
  members materialized versus distinct members read through the access paths
  (`Type::get_property_access_type`, `ObjectType::get_property_access_type`,
  call receiver member lookup).
- **P3, lazy member forces.** Add the missing
  `lazy_member_annotation_force_count`: creates versus forces under
  `SURGE_LAZY_IFACE_MEMBERS=1`.

Exit: record the numbers here. They size Stages 1 and 2. A P2 read ratio near
1 would end the laziness plan.

**Results** (2026-10-19, commit `888c19a2`, fast-profile
build):

- **P2.** About 97% of the members interface expansions materialize are never
  read. The access paths count `get_property_access_type`/`get_property_type`,
  and the relation paths count the member loops of `object_assignability_failure`
  and the subtype `Relater`. Direct `properties.get` reads elsewhere in the
  checker are not counted, so the true read share is somewhat higher.

  | corpus | methods materialized | methods ever read | properties materialized | properties ever read |
  | --- | --- | --- | --- | --- |
  | zod | 2.09M | 2.9% | 1.42M | 4.1% |
  | trpc | 1.72M | 1.7% | 2.30M | 1.9% |
  | drizzle-orm | 1.07M | 1.3% | 1.34M | 2.5% |
  | tanstack-query | 0.16M | 6.1% | 0.45M | 2.5% |

- **P1.**
  - Same-id (Ref, Ref) pairs pass on their arguments in 86% (zod), 80%
    (trpc), 99.2% (drizzle-orm) and 95% (tanstack-query) of cases.
  - Before that arm runs, `type_variable_related` peeled 0.65M (zod), 2.65M
    (trpc), 4.04M (drizzle-orm) and 1.52M (tanstack-query) references, and the
    weak-type check peeled 0.30M-2.42M targets.
  - On trpc, half of all lazy expansions (135k of 270k) were forced inside a
    relation.
- **Profile context.** `Type::peeled` is 22-34% of the check thread (inclusive)
  on these corpora. The never-returning-call check alone is 3.8% of trpc and
  8.9% of tanstack-query.

### Stage 1: reference-aware relations (members still eager)

Goal: a relation that the type arguments can decide peels nothing, and the
answers are unchanged.
- **1a. No premature peels.**
  - `type_variable_related` tests the other side's cheap condition before it
    peels.
  - The weak-type check is skipped for a same-id (Ref, Ref) pair. Both sides
    declare the same member names, so tsc's common-property check cannot fail
    there.
- **1a results** (commit `934985ba`). Besides the two items above, the
  never-returning-call check stopped peeling: an interface instantiation
  cannot be `never` (`ResolveReference::instantiates_interface`), and an
  expansion a reference already holds is read in place.
  - Method: one binary, `SURGE_REF_RELATIONS=0` versus on, 3 interleaved
    rounds, fast-profile build.
  - Instructions: trpc −2.6%, zod −1.8%, drizzle-orm −2.5%, tanstack-query
    −2.3%.
  - On trpc, `type_variable_related` peels fell from 2.65M to 586, and lazy
    expansions forced inside relations fell from 135k to 69k.
  - Diagnostics byte-identical on the 8 corpora, trpc `--jobs 8` identical to
    serial, `oracle:sweep --all` unchanged (919/923).
- **1b. Cache (Ref, Ref) verdicts.** Key them on `id` plus the pointer of the
  `arguments` slice, under the same assumption rules as the existing relation
  cache (no store after an assumption event).
- **1c. Identity without peeling.** In `is_type_identical_to` and the
  subtype `Relater`, a same-id pair with identical arguments is identical. The
  `Relater` also gets the assignability arm's argument-pass rule.
- **1d (separate gate). A final "no" as in tsc.**
  - Widen the variance encoding (4 bits per parameter, or u64) to add
    measured-covariant and independent codes.
  - Measure variance the way tsc does (markers, cached per declaration), so a
    failed argument check can be final.
  - This can change diagnostics, toward tsc. It lands only with fixtures, and
    only if Stage 2 needs it.

### Stage 2: lazy method components, all files

- Close the shell gaps first: parameter names, type-parameter head,
  polymorphic-`this` flags, declaration signature.
- Lift the library-only gate. Keep the one-member-group and check-phase limits
  until overload groups have content-addressed ids.
- Two lazy components with the same member key and substitution fingerprint
  are identical without peeling. The Stage 1 identity paths then answer member
  comparisons without forcing them.
- Return types stay eager until the 25 tRPC false positives are understood.

### Stage 3: lazy property members, all files

The old Stage 1, extended to user files. It relies on Stage 1 so that
relations do not force every member. Index infos stay eager, as in tsc.

### Stage 4: reference-level deferral

A user generic instantiation becomes a `LazyInstantiation` reference, as
library ones are, and heritage members merge lazily. Creating
`ZodEffects<X>` then costs nothing until a member is read.

## Risks

- **Display drift.** A lazy component renders its annotation text, not its
  resolved form. Run the sweep with `--strictMessages` as an early warning.
- **Relation answers.** Stage 1d changes them by design. Stages 1a-1c must
  not.
- **Cache rules** (CLAUDE.md):
  - Never cache degraded, `Unknown` or recursion-in-progress results
    program-wide.
  - Lazy references capture environments only as interned declaration
    environment handles.
- **Determinism.** Lazy forcing order must not leak into rendered text; check
  `--jobs 8` against `--jobs 1` at every stage.
