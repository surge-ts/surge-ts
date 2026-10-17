# Generic body returns read once and instantiated per call

> **Historical engineering report — a point-in-time record.** It documents one
> investigation as it stood on 2026-10-18 and does **not** describe current
> behavior, current counts, or current performance. For the current state see
> [CURRENT_STATUS.md](../../CURRENT_STATUS.md); for benchmark methodology see
> [BENCHMARKS.md](../../BENCHMARKS.md).

Follow-up to [CORPORA-WINDOWS-PROFILE-2026-10-17.md](CORPORA-WINDOWS-PROFILE-2026-10-17.md),
same machine and method (Windows 11, i7-12700K, benchmarked processes pinned to
the P-cores with power throttling off, 3 interleaved reps after a warm-up pair).

## The cost

An unannotated generic declaration's return comes from its body.
`instantiated_body_return` read that body again for every call, under that
call's bindings. On trpc the expensive reads are three test helpers
(`testServerAndClientResource`, `getServerAndReactClient`, `testReactResource`),
called once per test file with that file's router. Their bindings differ almost
every time (141 distinct of 251 calls for the first), so the per-binding cache
misses, and each read nests up to six body reads deep. Reading each body once per
declaration, with wrong results allowed, put a ceiling on the win: trpc 65.6 s →
36.8 s.

tsc reads the body once over the type parameters and instantiates the result per
call. surge could not, for two reasons. Its read over type variables degraded
(`IntersectionError<…>`, `{ [x: string]: any }`), and there was no type-level
`instantiateType`.

## What changed

- **A faithful generic read.** The body is read with its own type parameters
  bound as type variables (`enter_body_type_variables`), *and* with those names in
  `type_parameter_scopes`. Without the second, a written type argument such as
  `createTRPCReact<TRouter>()` resolved to a placeholder. Inside the read, an
  alias instantiation whose arguments are type variables stays a lazy reference,
  as one with concrete arguments does. It does not expand eagerly.
- **Origins on deferred types.** A deferred conditional, mapped or
  indexed-access type made during the read records its written node, the
  substitution and the declaring file (`infer/types/origin.rs`,
  `type_variable::set_deferred_origin`). A generic alias reference that resolved
  to a deferred body records the reference node instead, so the instantiation
  keeps the alias. References built over the variables keep their declaration
  (`origin_backed_reference`), and `LazyInstantiation` can rebuild itself
  (`ResolveReference::reinstantiate`). The fallback for other references is
  rebuilding the reference from its declaration by id.
- **`instantiate_type`** (`checks/function/generic_return.rs`) replaces the
  variables. A deferred type is re-resolved from its origin under the
  instantiated substitution, in the context of the file check that asked
  (the program context lacks that file's imports). A degraded re-resolution is
  kept, as the per-call resolution keeps it.
- **Validated switch-over.** A declaration's generic read is tried after 3
  per-call reads. It is trusted after its instantiation matched the per-call read
  twice: same rendering, and equal or equal up to the canonical store's interning
  ids. One mismatch distrusts it for the run. Variable registry entries are
  snapshotted with the read and restored while instantiating. The reads live in a
  thread-local map that is cleared in `clear_program_type_caches`.
- **Isolation.** Everything above is active only inside a generic body read.
  While a read is in progress, nothing is written to the program instantiation
  or persistent generic caches. Writing there let a read seed what a later
  ordinary resolution found, and that changed one message text on trpc.

`SURGE_GENERIC_RETURN_INSTANTIATE=0` turns the switch-over off.
`SURGE_GENERIC_RETURN_SHADOW=1` logs, per call, the per-call read against the
instantiated one, with the first structural difference.

## Measurements

Against the base `c54ee40d` (the change applied on top of it), 3 pinned
interleaved reps, medians:

| corpus | base | with the change | wall | CPU |
| --- | --- | --- | --- | --- |
| trpc | 61.6 s | 44.1 s | −28.4% | −27.6% |
| drizzle-orm | 35.5 s | 30.8 s | −13.3% | −13.7% |
| tanstack-query | 6.12 s | 6.04 s | −1.2% | −3.2% |
| zod | 21.7 s | 22.2 s | +1.9% | +1.8% |
| ts-pattern, zustand, ofetch, ky | | | within ±4% | |

drizzle-orm's base reps were 30.6 / 40.2 / 35.5 s, so its gain is within noise.
Before the "3 per-call reads first" threshold, zod came out at +5.5%: it paid for
generic reads of declarations that never matched. Peak RSS moved by at most
1.2% (zustand, 249 → 252 MB; trpc +0.6%). In a non-LTO build, the same switch on the base took trpc from
73.9 s to 53.1 s.

Diagnostics were byte-identical to the base on all 8 corpora. `oracle:sweep --all`
was 916/922 with and without the switch, with identical failure and drift lines.
`vitest run tests` had the same 53 failures (all Windows/environment) with and
without it.

## Found along the way

- **trpc `initTRPC.ts:124` TS2344 depends on heap layout.** The base binary
  reports it or not depending on one extra allocation: setting an unused
  `SURGE_LAZY_VALUE_TRACE` value flips it. tsc does not report it. Treat it as
  noise in any byte comparison of trpc output.
- Function and object equality in surge include canonical-store ids
  (`parameter_list_id`, property-map pointers in the canonical fingerprint). A
  rebuilt type equal in content compares unequal, so the validation compares
  structure below the ids.
- Deferring an indexed access whose object is a caller's type variable matches
  tsc on a repro. Applied to ordinary checking, though, it adds a trpc false
  positive (`httpSubscriptionLink.ts:202`), so it is confined to generic reads.

## Risks and limits

- The trust decision is per worker thread and per run. Under `--jobs N` with
  several workers, a declaration can be trusted on one worker and not another. A
  trusted instantiation that would differ on a later call is not caught; the
  byte-identical corpora are the evidence that this does not happen on them.
- zod and drizzle-orm do not benefit: their per-call reads are cheap or do not
  match, and the validation reads cost a little.
- The remaining trpc time is mostly the binding passes before check (see the
  2026-10-17 report). The 5 s target is not reached.
