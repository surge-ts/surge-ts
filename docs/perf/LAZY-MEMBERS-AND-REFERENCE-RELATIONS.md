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
| Stage 2a: defer interface-instantiation method return types | zod +6.1% instructions, 2 new false positives. Forcing a deferred return re-resolves the written annotation, which costs more than the eager resolution it replaced |
| G1: read each generic interface once over type variables and build `I<A>` with `instantiate_type` (see below) | zod 139.5 G → 1,752 G instructions and 23 new TS2320, with only 15% of expansions eligible |
| Cache each written conditional and mapped node's resolution per substitution and context (see prerequisite 3 below) | Diagnostics identical, but instructions +6.6–9% on all four corpora: 72% of zod's misses are first-time pairs, and the repeats are split by the environment store and other context the key must carry |

### G1: one generic read per interface (2026-10-19, rejected)

Prompted by comparing surge with ts-rust (a Rust port of tsgo): tsc resolves a
generic interface once over its own type parameters and makes `I<A>` by
instantiating that resolution through a mapper. The prototype did the same
behind `SURGE_GENERIC_IFACE`:
- It read the body once with type variables, in the declaration's own scope,
  with the caller's resolution state cleared.
- It used the generic-return machinery: `enter_body_type_variables`, deferred
  origins and `instantiate_type`.
- In shadow mode it compared every eligible expansion with the read's
  instantiation. In `1` mode the read stood in for a declaration after two
  matches.

Inside a read, user interfaces instantiated over the variables were kept as
lazy references. Expanding them re-entered the declarations being read
through their heritage, and the back-edge over variables degraded the read.

Measured on zod (same binary, fast profile):

| | gate off | `SURGE_GENERIC_IFACE=1` |
| --- | ---: | ---: |
| instructions | 139.5 G | 1,752 G |
| diagnostics | 89 | 112 (+23 TS2320 `'~standard' ... not identical`) |

Counters with the gate on:
- Expansions: 55,303 eligible, about 15% of the roughly 360,000 interface-body expansions. The rest bind a placeholder, a degraded argument, the sentinel or a type parameter, because they happen while a generic *signature* is resolved.
- Reads: 1,097, of which 138 were unclean.
- Comparisons: 1,451. 1,059 matched, 327 mismatched, 65 were impure expansions.
- Outcome: 405 of 1,452 declarations trusted, 27,233 expansions replaced.

Why it failed:
- **Reads are not cheap in surge.** A read peels lazy references whose
  arguments are variables: heritage, and conditional types deciding on a
  check type. Each peel expands that body again. Program caches are closed to
  generic reads, so nothing is reused. One zod read also bound 32,000+ distinct
  deferred conditionals in a linear registry, until a per-read cap of 256 was
  added.
- **Instantiations differ from the expansion** in ways an exact comparison
  cannot pass:
  - A namespace-qualified spelling (`core.$ZodCatchInternals`) reaches a
    rebound `InterfaceInfo` whose reference id differs from the unqualified
    declaration's. This was more than half of the mismatches, and it is
    behind the new TS2320s: identity compares ids.
  - Some per-substitution expansions are themselves degraded: `version:
    typeof version` became `unknown`, and a heritage cycle dropped `_ZodType`'s
    `def`.
  - Some reads leave a deferred `T["errors"]` unresolved, or turn
    `Promise<Result>` into `unknown`.
- **The ceiling is low.** Even with free reads, only the eligible 15% could be
  replaced, so at best about 5% of zod's time.

What it would take:
1. One identity per declaration, however it is spelled.
2. An instantiation cache that works over type variables. That needs
   variables with a stable identity, not entries in a thread-local registry.
3. Conditional, mapped and indexed-access evaluation that defers on a generic
   operand instead of peeling it.

The larger lever is the signature-level read (zod's
`instantiate_declared_member_signature` is 43% of the check thread), and it
needs the same three things.

The prototype diff was not kept in the tree.

### Prerequisite 3: deferred conditional and mapped types (2026-10-19)

Prerequisites 1 and 2 landed as `be0bd975` (one identity per declaration) and
`0483a300` (stable type variables). This section is the third. All numbers are
from one session on branch `perf/reference-relations`, fast-profile builds
unless noted.

**Sizing.** A probe in `resolve_conditional_type` (not kept) counted:

| | zod | drizzle | trpc | tanstack |
| --- | ---: | ---: | ---: | ---: |
| conditional resolutions | 287,465 | 631,023 | 328,799 | 86,533 |
| deferred over a generic operand | 4,011 | 29,516 | 4,974 | 375 |
| substitution binds a type variable | 6,182 | 78,385 | 20,663 | 1,636 |
| `resolve_conditional_type`, inclusive | 18.9% | 49.7% | 14.3% | 15.7% |
| deferral (`deferred_generic_conditional`), inclusive | 0.4% | 5.0% | — | — |

The cost is concrete evaluation, not deferral. In drizzle, 82% of conditional
time re-resolves the written check, extends and branch nodes under the
substitution. The type-variable registry is small in these runs: an
`is_active_variable` lookup scans 9–23 entries on average, at most 217.

**Rejected: an instantiation cache per written node.** tsc caches a
conditional's instantiations on its root, keyed by the outer type arguments.
The surge equivalent keyed each conditional and mapped node on:
- the substitution;
- the resolution context: file, scope, namespace prefixes, type-parameter
  and constraint scopes, `this`, phase, environment store, declaration table
  content, frames, the resolution stack, the lazy-peel stack and the
  array-constrained signature parameters.

It replayed the effects a resolution has outside its result: expansion and
instantiation budgets, depth reached, cycle indices, and the cells an
interface expansion publishes. It kept only clean, diagnostic-free results,
per file check.
- Diagnostics were identical on all four corpora.
- Instructions rose: zod +9%, drizzle +7.6%, trpc +6.6%, tanstack +7%.
- Hits were zod 5,000 of 360,000 lookups, drizzle 63,500 of 630,000 and
  trpc 1,700 of 417,000.
- Why: 72% of zod's misses were the first time that node met those
  bindings. The repeats were split by context the key must carry. Each row
  below counts misses where an entry for the same node and bindings existed
  but the named field differed.

  | context field that differed | zod | drizzle | trpc |
  | --- | ---: | ---: | ---: |
  | environment store | 32,902 | 60,064 | 64,246 |
  | cross-file depth | 3,732 | 33,817 | 5,701 |
  | signature parameter bindings | 4,024 | 30,728 | 11,734 |

  The rules forbid environment-insensitive caches. A probe that keyed only
  node and bindings had suggested repeat rates from 12% (tanstack) to 87%
  (drizzle). That figure does not survive the context surge's resolution
  actually reads.

**Landed: type-level instantiation of a deferred conditional.**
`generic_return::instantiate` evaluates a deferred conditional itself instead
of resolving its written node again through the origin:
- It instantiates the check and extends types.
- It decides by assignability, distributing over a union check type.
- It instantiates the chosen branch with the check parameter bound to each
  member. The true branch's implied `T & extends` becomes the member once
  the member satisfies it, as tsc's substitution type does.

It hands back to the origin for `infer` captures, `any`, an unmodelled
operand, an operand still generic, or a branch that does not instantiate
cleanly. Once their operands are concrete, intersections over variables are
merged rather than kept as operands. Gate: `SURGE_TYPE_LEVEL_DEFERRED=0`.
Today it is reached only from generic return reads: trpc decides 50
conditionals, zustand 154, the other corpora none.

A first version let a part it could not instantiate fail the whole
instantiation instead of handing back to the origin. That happened 146 times
on trpc. The cost lands on generic return reads, because one failed
comparison distrusts a read for good and every later call reads the body
again. `getServerAndReactClient` lost its read that way, which cost trpc
+10.4% instructions with identical diagnostics. Unsupported parts now go back
to the origin, and so does a branch that still closes over a variable behind
a lazy reference. With that, trpc's trust counts are the same with the gate
off and on: 19 reads validated, 11 distrusted.

Same LTO binary, built from the tree this section was committed with, gate
off and on:
- The 8 corpora are byte-identical, and so is trpc at `--jobs 8`.
- `oracle:sweep --all` passes 919 of 923 either way, and no target differs.
- vitest: 3,751 of 3,770 pass either way, with the same 16 failures.
- Instructions, medians of three interleaved rounds:
  - zod 86.16 → 85.99 G
  - drizzle 136.99 → 136.98 G
  - trpc 175.30 → 174.77 G
  - tanstack 39.79 → 39.79 G

  That is noise, and peak memory stays within 6 MB. The evaluator is the
  foundation, not a speed-up: its reach today is generic return reads.

**Shadow measurement.** With `SURGE_DEFERRED_SHADOW=1` and
`SURGE_LAZY_PROBE=1`, each conditional or mapped node resolved under
concrete bindings is read again with those bindings replaced by type
variables. That read is instantiated at the type level and compared with
the real result. `SURGE_DEFERRED_SHADOW_CAP` bounds the number of distinct
node and binding pairs. Without a cap, an earlier version of the harness
reached 60–64 GB on zod and trpc: every shadowed node gets variables of its
own, and the expansions over them are retained. That version also overflowed
drizzle's 512 MB check-thread stack at a 20,000 cap. The runs below use the
fixed harness at the caps shown, and all of them completed, peaking at
0.9–4.8 GB. Conditionals, per distinct node and bindings:

| | zod (3k) | drizzle (5k) | trpc (20k) | tanstack (20k) |
| --- | ---: | ---: | ---: | ---: |
| shadowed | 2,160 | 4,690 | 17,277 | 19,881 |
| read over variables degrades to `unknown` | 1,455 (67%) | 67 (1%) | 12,859 (74%) | 19,019 (96%) |
| read defers | 683 | 4,623 | 4,396 | 861 |
| decided at the type level | 317 | 2,128 | 2,335 | 206 |
| of those, equal to the real result | 100 | 1,596 | 1,261 (+209 display only) | 143 |

What the type level left undecided:
- trpc: 1,303 branches that did not instantiate cleanly, 510 `infer`, 413
  parts the walk cannot instantiate.
- drizzle: 1,278 `infer`, 1,053 unmodelled members.

Mapped reads that deferred were almost all undecided. trpc decided 121 of
its 1,398, through conditionals inside them. Mapped types themselves have no
type-level instantiation yet.

Where they disagree, from trpc's log:
- An alias reference in a branch stays a lazy reference where the written
  path resolves the alias: `Prettify` (174), `PickTypes` (76),
  `QueryProcedure` (58), `MockParameters` (31).
- Alias-display arguments render differently: `DecoratedProcedureUtilsRecord`,
  `DecorateRouterRecord` and `DecoratedProcedureRecord`, 185 together.
- A property exists only in the real result (120).
- A nested part comes back `unknown` where the real result has `void` (107),
  or `any` where it has `false` (45).

The degraded reads are concentrated:
- tanstack: `@vitest/expect` (13,587) and `@vitest/spy` (5,117).
- trpc: `@vitest/expect` (8,462), zod's core `schemas.d.cts` (2,036),
  `router.ts` (1,111) and `types.ts` (406).
- zod: `v4/core/schemas.ts` (1,081) and `v4/classic/schemas.ts` (537).

What it means for lazy members: forcing a member as a type-level
instantiation of a once-read declaration is blocked on the read side, not
the instantiation side.
- Most conditional nodes outside drizzle cannot be read over variables
  without degrading.
- Where they can, type-level evaluation still needs:
  - `infer` matching on types;
  - eager re-resolution of alias references;
  - mapped-type instantiation.

### Read-side degradation (2026-10-19)

The shadow now tags every exit to the `unknown` sentinel with a reason
(`note_degrade`). It records the first and last reason during a read, and
the node's written shape. It also skips a real result that is degraded
inside, with a member already `unknown`, because no read over variables can
reproduce that.

**Cause.** Nearly every degraded conditional read (`check_sentinel`, with
an indexed-access check type such as `O[K] extends …`) came from one place.
`resolve_indexed_access_type` deferred `T[K]` only when nothing was
substituted, or inside a generic body read. A substitution that binds the
operands to type variables counts as an instantiation, so `O[K]` with `O`
bound to a variable fell through to the "receiver resolved to unknown" exit
(`is_unknown` is true of a type variable).

tsc's `instantiateType(T[K], T → U)` is `U[K]`. The same gap hit ordinary
checking: inside `read<U extends { a: number }>(u: U)`, a call to
`get<T, K extends keyof T>(obj: T, key: K): T[K]` returned `unknown`, and
tsc's TS2322 on assigning it to `string` went missing.

**Fixes.** Gate: `SURGE_DEFER_OVER_VARIABLES=0`.
- An indexed access whose substituted object or index is a type variable
  stays a deferred `T[K]` in every context.
- The template and `as` clause of a mapped type over a generic key set now
  resolve in a lazy member frame, as a literal key's template already did.
  tsc instantiates a mapped template on demand, so an alias re-entering
  itself through it (vitest's `ChainableFunction`) is legal recursion.

That exposed one false positive in trpc, `httpSubscriptionLink.ts:202`.
`chunk.event && …` kept the left operand's type variable as its falsy
part. tsc's `getDefinitelyFalsyPartOfType` is `never` for a type variable,
so `x && y` over `x: T` is `y`'s type. The `&&` arm now uses
`definitely_falsy_part`. This fix is not gated. Against the previous commit
it leaves the 8 corpora and the sweep unchanged, and it removes three false
TS2322s in the fixture. `||` still asks whether a type variable may be
falsy, which it may.

Shadow, same binary, gate off → on (caps as before, distinct node and
bindings):

| | conditional reads degrading | decided type-level, agreeing | mapped reads degrading |
| --- | ---: | ---: | ---: |
| zod | 1,452 (66%) → 5 (0%) | 52% → 91% of 1,627 | 303 (39%) → 7 (1%) |
| drizzle | 69 (1%) → 14 (0%) | 90% → 94% of 2,115 | 43 (13%) → 11 (3%) |
| trpc | 14,360 (80%) → 14 (0%) | 42% → 88% of 15,749 | 920 (47%) → 54 (3%) |
| tanstack | 19,115 (96%) → 3 (0%) | 67% → 94% of 16,909 | 12 (38%) → 0 (0%) |

Same LTO binary, gate off and on:
- The 8 corpora are byte-identical, and so is trpc at `--jobs 8`.
- `oracle:sweep --all` passes 919 of 923 either way, and no target differs.
- vitest: 3,753 of 3,772 pass with the gate on, with the same 16 failures as
  before. With it off, the new `U["a"]` fixture fails too.
- Instructions, medians of three interleaved rounds:

  | | off | on | change |
  | --- | ---: | ---: | ---: |
  | zod | 86.27 G | 86.70 G | +0.5% |
  | drizzle | 137.58 G | 142.66 G | +3.7% |
  | trpc | 176.56 G | 177.28 G | +0.4% |
  | tanstack | 39.88 G | 39.88 G | 0.0% |

  The drizzle cost is the indexed-access deferral, not the template frames,
  which cost nothing when switched off alone. A generic body now carries a
  real `T[K]` where it carried the sentinel, which is tsc's type. Drizzle's
  counters, gate off → on:
  - deferred types created: 24,445 → 37,619;
  - interface expansions: 141,751 → 149,579;
  - variable-memo misses: 13,438 → 19,374;
  - same-id reference relations falling through to a structural comparison
    for want of variances: 2,307 → 7,819.

What remains:
- **Residual degrades.** trpc has 26 mapped types over a constraint with no
  literal keys, and drizzle has 11 `Equal<…>`-idiom conditionals
  (`identity_undecidable`).
- **Results already degraded inside.** These are skipped, and there are far
  more of them than shadowed reads: drizzle has 33,880 conditional and
  63,330 mapped calls, zod 18,192 and 12,743. In zod's `partial<M extends
  util.Mask<keyof Shape>>`, the method's own `M` is bound to the sentinel
  when its signature is resolved, so `keyof M` is `unknown` with no variable
  involved. That is the signature-level blocker: a generic signature's own
  type parameters are erased to the sentinel instead of read as variables.
  See "Signature-level variables" below.
- **Undecided at the type level.** Two reasons lead the hand-backs to the
  origin: `infer` patterns and parts the walk cannot instantiate.
  - zod: 376 `infer`, 191 cannot instantiate.
  - trpc: 1,104 `infer`, 854 cannot instantiate.
  - tanstack: 797 `infer`, 2,066 cannot instantiate.
  - drizzle also has 1,053 unmodelled members.

  Mapped types still have no type-level instantiation.

### Signature-level variables (2026-10-20)

**Blocker.** `extend_substitution_with_type_parameters` binds a generic
signature's own type parameters to the sentinel or a placeholder while the
signature resolves, so what is written over them (`keyof M` in zod's
`partial<M extends util.Mask<keyof Shape>>`) is `unknown` before any call.
tsc reads `<M>(mask: M) => R` over `M` itself and instantiates it per call
with the signature's mapper.

**In place: rejected, opt-in.** `SURGE_SIGNATURE_VARIABLES=1` binds them to
the declaration's type variables (the ones a body being checked binds)
while the erased signature resolves. Guards keep the reads lazy:
- a reference whose arguments are built from them stays a lazy reference;
- `keyof` and `[index]` over such a reference defer;
- a conditional over it counts as generic and does not distribute.

Without the guards, tanstack and zod ran away expanding every member over
the variables. Each generic method enters variables of its own. With the
guards, three corpora finish but still lose, and drizzle overflows the
checker thread's stack. Same clean LTO build of `88c3f93c`, default → on,
one run each:

| | instructions | errors only in place | tsc reports them |
| --- | ---: | --- | --- |
| tanstack | 40.05 G → 95.59 G (+139%) | 1 TS2769 | no |
| zod | 87.68 G → 216.63 G (+147%) | TS2322, TS2345 | no |
| trpc | 177.90 G → 325.72 G (+83%) | TS2322, TS2353, 2 × TS18046 | no |
| drizzle | 142.79 G → stack overflow | — | — |

On trpc, seven TS2339 messages render the generic method's parameter over
its variable instead of `unknown`. tsc names the class instead, so this is
message drift either way.

The erased signature is also related, not only instantiated: overload
resolution, assignability of a generic function type, the declaration's own
checks. The sentinel made those comparisons permissive, and over variables
they expose divergences it masked. One example is zod's `or()`, where the
array literal returned under a generic contextual signature gets no tuple
context. tsc reports none of the new errors. So the variables have to stay
out of the erased signature until everything that relates it is ready.

**On the side.** `SURGE_SIGNATURE_SHADOW=1` (with `SURGE_LAZY_PROBE=1`)
sizes G3: read the signature once, instantiate it per call. At each call
that resolves a written signature again under its inferred bindings, the
shadow does two things:
- It resolves every written parameter and the return over the signature's
  variables.
- It instantiates that generic form at the type level with the same
  bindings, then compares it with the call's own result.

It skips a call when any of its bindings is the sentinel, a placeholder, or
degraded.

The shadow is not inert. Its reads over variables fill caches that the run
then uses:
- trpc: one more TS4111, and expect-type TS2349s move (two gone, one new);
- tanstack: two more expect-type TS2349s;
- drizzle: two messages render a column builder expanded instead of
  through its alias;
- zod: no change.

Its counts size G3, but its diagnostics are not the default's.

The comparison is a structural walk. It expands a reference that either
side kept lazy, descends tuples, and matches union members in any order.
It stays apart from `first_difference`, because the generic-return trust
check relies on that narrower walk.

The shadow surfaced three instantiation gaps, now fixed:
- **Dead deferred types.** A deferred type is registered only while the
  variables it is built on are bound. Stable owners make a signature's
  variables equal across its entries, so a read cached during one entry
  came back in the next with deferred types that were no longer registered.
  Instantiation then passed over them as opaque (`Writable<T>[number]` in
  every drizzle `text()` call). Each signature's registry is now
  snapshotted when its variables are released, and restored when they are
  bound again (`SIGNATURE_SNAPSHOTS`, cleared in
  `clear_program_type_caches`). It fills only while signature variables
  are bound.
- **Access over a reference.** A deferred `reference[index]` or
  `keyof reference` instantiates through the reference
  (`instantiate_over_reference`). `substitute_variable` cannot rebuild a
  reference.
- **A tuple read by `number`.** `[A, B][number]` and `[A, ...B[]][number]`
  read to the element union. `indexed_access_lookup` had no tuple arm.
  `substitute_variable` reads through the same function, so this reaches
  the default path too. The corpora and sweep below show it changes no
  diagnostic.

Shadow at `88c3f93c`, clean LTO build. Counts are per resolution of a
generic signature at a call. A call can resolve its signature more than
once while inferring. "Same" is the share that agree structurally among
the components the call itself read clean. "Rendered differently"
counts returns that agree structurally but print differently. Disagreeing
and degraded counts are parameters + returns.

| | resolutions | skipped, unbound | parameters same | returns same | returns rendered differently | disagree | generic read degraded |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tanstack | 30,937 | 14,800 | 98.7% of 19,489 | 99.6% of 15,362 | 9,585 | 144 + 4 | 95 + 39 |
| zod | 22,388 | 2,898 | 98.4% of 28,612 | 96.5% of 13,141 | 572 | 21 + 11 | 412 + 379 |
| drizzle | 36,644 | 7,341 | 99.6% of 44,568 | 98.3% of 24,531 | 17,432 | 167 + 248 | 2 + 173 |
| trpc | 40,043 | 8,269 | 97.9% of 27,628 | 96.1% of 24,160 | 8,829 | 500 + 272 | 72 + 357 |

The rest of each total is undecided: a deferred form the walk cannot
instantiate. That is at most 302 per corpus, and trpc's returns are the
largest case. Resolutions of signatures with no type parameters of their
own are not compared.

Where the call's own read degraded, the generic form instantiated clean
for some parameters and returns:

| | parameters | returns |
| --- | ---: | ---: |
| tanstack | 105 of 831 | 4 of 648 |
| zod | 182 of 3,647 | 426 of 3,545 |
| drizzle | 7 of 489 | 235 of 821 |
| trpc | 419 of 5,229 | 358 of 5,113 |

What remains before the generic form can replace the per-call read:
- **Instantiation gaps.** See "Residual instantiation gaps" below.
  - Some references expand to a lazily merged intersection
    (`HTTPBatchLinkOptions<TRouter["_def"]["_config"]["$types"]>`). The
    variable walk cannot see into the deferred merge, so the generic form
    comes back unchanged. This is about half of trpc's disagreements.
  - A conditional alias reference with a concrete check type
    (`DefaultValue<UnsetMarker, $Output>`) stays a lazy reference over the
    variables. tsc decides it at the declaration. Once instantiated, it
    expands right only on a signature's first call. That is the same
    alternation the dead deferred types showed, so it is most likely
    another cache keyed on stable variables.
  - expect-type's `Expected & AValue` merges the variable into
    `{ [x: string]: any; … }`.
- **Display.** Many agreeing results render differently. Aliases keep their
  written arguments (`Dispatch<SetStateAction<S>>`), and lazy references
  render as written. G3 needs display parity before its types reach a
  message.
- **Unbound calls.** These are not compared, and are many where inference
  leaves the sentinel.

Default configuration, against cb9dbafa, clean LTO builds of both:

- The 8 corpora are byte-identical, and so is trpc at `--jobs 8`.
- `oracle:sweep --all` passes 919 of 923 either way, and no target differs.
- vitest: 3,753 of 3,772 pass on both, with the same 16 failures. The
  cb9dbafa side ran on a clean fast-profile build.
- Instructions, medians of three interleaved rounds:

  | | cb9dbafa | 88c3f93c | change |
  | --- | ---: | ---: | ---: |
  | zod | 86.88 G | 86.84 G | −0.05% |
  | drizzle | 142.83 G | 142.71 G | −0.09% |
  | trpc | 177.68 G | 177.99 G | +0.17% |
  | tanstack | 40.03 G | 40.07 G | +0.09% |

  Every change is within run-to-run spread. Under other sessions' load,
  trpc's rounds span 4.6 G. Peak footprint moves by 4 MB at most.

Next, after the gaps above: use the generic form at call sites behind a
trust check, as generic return reads do (`validate_generic_read`). That
means reading once per signature, instantiating per call, and falling back
for good on the first mismatch.

### Residual instantiation gaps (2026-10-20)

The three gaps listed above came down to four instantiation rules, plus two
flaws in the shadow itself. Commit `e370f02f`. Gate:
`SURGE_INSTANTIATION_REBUILD=0`.

**Shadow.**
- It instantiated outside the file check's instantiation context. Generic
  return reads already wrap theirs in `with_instantiation_context`.
  Without that context, a rebuilt reference re-read its alias body in the
  program context, which lacks the file's values. `typeof unsetMarker` failed
  there, and `DefaultValue<UnsetMarker, X>` expanded to `unknown`.
- It compared same-id references by their arguments alone, so an instance
  that expanded to the sentinel still counted as agreeing. It now also
  requires the instance to expand where the call's does. Under that check,
  the earlier build's drizzle returns agree 80.5% with the gate off. Most of
  what `88c3f93c` counted as agreeing were rebuilt references expanding to
  `unknown`.

**Instantiation.**
- **An intersection is intersected again.** tsc instantiates every
  constituent and calls `getIntersectionType` again. surge merged once and
  kept only the operands the merge could not take in. Instantiating then
  replaced those operands but left the members merged before. An
  intersection whose operands name the variables is now re-merged from its
  instantiated operands:
  - the open index that stood for a variable's members is dropped;
  - an alias operand that expands to a union distributes the intersection,
    as the alias resolved in place does;
  - a member the merge had read off an operand is not restated beside it,
    since `(string | URL) & (string | URL)` does not reduce when the two
    unions are not the same value.
  This is trpc's `HTTPBatchLinkOptions<TRouter["_def"]["_config"]["$types"]>`.
- **An instance expands where its reference does.**
  `LazyInstantiation::reinstantiate` built the instance in the instantiation
  context, or the program's, and that context cannot read the declaring
  file's values. It now prefers the environment of the reference it
  instantiates.
- **A reference rebuilt from its declaration binds its arguments.**
  `reference_from_declaration` now builds in the instantiation context. Its
  resolved arguments now bind even with none written: before, a generic
  interface fell to the arity check and expanded to `unknown`.

Default configuration, same binary, gate off → on:
- The 8 corpora are byte-identical, and so is trpc at `--jobs 8`.
- `oracle:sweep --all` passes 919 of 923 either way, and no target differs.

Shadow at `e370f02f`, clean LTO build, gate on (off):

| | parameters same | parameters disagree | returns same | returns disagree |
| --- | ---: | ---: | ---: | ---: |
| tanstack | 98.7% (98.7%) | 138 (144) | 99.6% (99.3%) | 2 (46) |
| zod | 98.5% (98.4%) | 17 (29) | 96.5% (96.0%) | 13 (78) |
| drizzle | 99.9% (99.5%) | 26 (198) | 98.3% (80.5%) | 240 (4,602) |
| trpc | 99.1% (97.7%) | 153 (546) | 96.4% (94.6%) | 188 (654) |

What remains:
- **Expanded generic forms.** trpc's `QueryProcedure<{ … output:
  DefaultValue<UnsetMarker, $Output> … }>` (115 returns) reads as an expanded
  interface object, not a reference, and its instance still disagrees. A
  smaller repro with the same declarations reads it as a reference, which
  agrees.
- **`T & Disposable`.** The generic read drops the variable beside a lone
  reference operand: `with_type_variable_operands` has no arm for one, and
  the fallback returns the merged reference. This is trpc's
  `makeResource<T>(…): T & Disposable`. The same merge serves generic body
  reads, so the fix touches the default path.
- **expect-type's `Expected & AValue`.** Instantiation now distributes it
  over `AValue`'s union, as `getIntersectionType` does. The call's own read
  keeps `{ foo: string } & AValue` whole, so the call is the side that
  differs.
- **Display.** This is unchanged: 17,289 of drizzle's 24,006 agreeing
  returns render differently.

Default configuration against `88c3f93c`, clean LTO builds of both.
`930999b6` changed only comments and docs.
- The 8 corpora are byte-identical, and so is trpc at `--jobs 8`.
- `oracle:sweep --all` passes 919 of 923 either way, and no target differs.
- vitest: 3,753 of 3,772 pass on both, and no test differs.
- Instructions, medians of three interleaved rounds:

  | | 88c3f93c | e370f02f | change |
  | --- | ---: | ---: | ---: |
  | zod | 87.21 G | 87.22 G | +0.01% |
  | drizzle | 143.84 G | 143.61 G | −0.16% |
  | trpc | 178.42 G | 179.46 G | +0.58% |
  | tanstack | 40.21 G | 40.30 G | +0.23% |

  Every change is within run-to-run spread. Under other sessions' load,
  trpc's rounds span 8.4 G. Peak footprint moves by 4 MB at most.

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
