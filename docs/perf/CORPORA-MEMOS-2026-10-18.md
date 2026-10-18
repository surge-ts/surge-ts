# Corpora check time, continued: validation up to laziness and six memos

> **Historical engineering report — a point-in-time record.** It documents one
> investigation as it stood on 2026-10-18 and does **not** describe current
> behavior, current counts, or current performance. For the current state see
> [CURRENT_STATUS.md](../../CURRENT_STATUS.md); for benchmark methodology see
> [BENCHMARKS.md](../../BENCHMARKS.md).

Follow-up to [GENERIC-RETURN-INSTANTIATION-2026-10-18.md](GENERIC-RETURN-INSTANTIATION-2026-10-18.md),
on the same machine with the same method: Windows 11, i7-12700K, LTO builds,
benchmarked processes pinned to the P-cores with power throttling off, and 3
interleaved reps after a warm-up pair. Each change was measured against its
parent. Every one kept diagnostics byte-identical on all 8 corpora and
`oracle:sweep --all` at 916/922 with identical failure and drift lines.

## Where trpc stands

| commit | change | trpc | other corpora |
| --- | --- | --- | --- |
| 9a4a14d3 | (start: lazy body-return degraded memo) | 33.2 s | |
| dfb83ed1 | generic-read validation up to laziness | 27.1 s (−18.6%) | ±1.5% |
| 69518f88 | spelling-suggestion memo | 25.4 s (−4.6%) | ts-pattern −41.5%, zod −7.8%, tanstack-query −5.9% |
| e4eb6f97 | conflict-digest property-fold memo | 24.2 s (−5.0%) | zod −14.1% |
| 9b1851f0 | `mentions_type_variable` property-walk memo | 24.0 s (−3.0%) | zod −3.0% |
| 4b5db5bc | declared-variance memo | 23.9 s (−0.7%) | drizzle-orm −3.0%, ts-pattern −2.6% |
| f790f19c | finished context left to process exit | 23.4 s (−2.3%) | every corpus −1.7 to −3.8% |
| 668db32c | environment key names its table by the shared map | 23.1 s (−1.8%), RSS −4.0% | zod RSS −5.3% |

Numbers before a row's change come from that row's A/B, so the column is not
one series of runs; the trpc chain is 33.2 s → about 23 s.

## What changed

- **Validation up to laziness (dfb83ed1).** testServerAndClientResource was
  distrusted on its first validation. The per-call read had expanded a
  contextually typed parameter's `OperationResultEnvelope<…>`, and the
  instantiation had kept it as a lazy reference. The two render alike and the
  expansion is equal. Display fingerprints hash a reference by id and display
  but an object by its properties, so the two forms never fingerprint-equal.
  The comparison now resolves a lone reference and compares its expansion, and
  it accepts an equal rendered `name()`. Instantiated signatures also carry
  their alias instantiation.
- **Spelling-suggestion memo (69518f88).** A dependency's unresolvable type
  reference fails again on every resolution of its declaration. Each failure
  measured a Levenshtein suggestion against every declared name: 8.0K
  measurements over 1.8K distinct inputs on trpc. The key is the name, the
  declaration and ambient maps (pinned by a `Weak`), the type parameters in
  scope and the namespace prefixes.
- **Conflict-digest and variable-walk memos (e4eb6f97, 9b1851f0).**
  `type_conflict_digest`, which runs for every instantiation-cache lookup, and
  `mentions_type_variable`, which runs for every interface resolution, walk
  every property of every object they reach. Nested objects multiply that
  walk. Each per-object property part depends only on the map and the depth, so
  it is memoized by (map address, depth), with the map pinned by a `Weak`.
- **Declared-variance memo (4b5db5bc).** Every reference to a generic
  declaration re-walked the declaration body to measure its variances. The memo
  key is body, resolution scope, name and strictNullChecks. An answer that fell
  back to the caller's scope records the names it looked up, and a later caller
  reuses it only when its scope finds the same declarations.
- **Teardown (f790f19c).** The fast-exit path already left the program state
  to process exit. It now leaves the checker context too.
- **Environment identity (668db32c).** Every clone of a table renews
  `(instance_id, version)`, and a context recovered from an environment clones
  its table. So each lazy reference made in a recovered context interned an
  environment of its own: 241K distinct keys on trpc, and 13.7K without that
  component. The key now names the table by the address of the map it shares.
  A captured snapshot holds that map, so it can neither change in place nor be
  reused while the key lives.

The memos follow one pattern. They are keyed by the address of an `Arc` that
changes only through `Arc::make_mut`, and pinned by a `Weak`. `make_mut` moves
a map that has a weak reference to a new allocation, and the pinned block cannot
be reused. They are capped and cleared with the program type caches.

## Measured and rejected

| idea | result |
| --- | --- |
| Re-resolving a variable-argument alias reference from its written node at instantiation | Shadow match counts unchanged on all of trpc |
| Not using a trusted instantiation inside another declaration's generic read | getServerAndReactClient became distrusted |
| A Windows `FindFirstFileExW` leaf probe for canonicalization (the macOS shortcut) | Exact on 22K paths, but ±1%: the kernel time the sampler showed was not avoidable |
| `SURGE_IFACE_CACHE_ALL`, `SURGE_LIB_MEMBER_CACHE` | +1–3% |
| `SURGE_LAZY_IFACE_MEMBERS` | trpc 24 s → 104 s and diagnostics drift. 77% of the time is `Relater::object_members_related` peeling lazy members |
| `SURGE_EARLY_MODULE_LOCAL_VALUES=0` | drizzle-orm 27 s → 257 s, trpc +24%, diagnostics drift |
| The preliminary scope map installed for the early local-value build | Fixes the unknown imported names in a repro, but trpc time unchanged |
| Pre-sizing the expansion's property map | Time within noise, RSS −1% |

## What is left

On trpc, the program memo stored 186K interface expansions. Expansions that
degrade (`ZodType`, `BaseType`, `ArrayType`, …) are never stored and run again.
That is about 30 s inclusive, with nesting counted twice. The rule against
caching degraded results program-wide stands, so the lever is making
expansions not degrade. Part of it is the preliminary pass. It builds class
value symbols (zod v3's static members) before the file's imports are bound,
so every imported name in those members fails. Part of it is the depth limit
that eager whole-interface expansion runs into. tsc instantiates members only
when read.

The current profile: check 51%, module-local values 17%, bind 12%, loader 6%,
preliminary 6%. `resolve_interface` is under 56% of all samples. The 5 s
target is not reached.
