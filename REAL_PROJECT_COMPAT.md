# Real Project Compatibility

How `surge-ts` is measured against real projects: which corpora exist, how to
obtain and run each one, and what a result on each one does and does not mean.

**This document carries no counts.** The current per-corpus numbers live in
[CURRENT_STATUS.md § Real-project compatibility](CURRENT_STATUS.md#real-project-compatibility)
and nowhere else; the confirmed gaps live in
[CURRENT_STATUS.md § Known limitations](CURRENT_STATUS.md#known-limitations).
The per-corpus burn-down logs this file carried until 2026-10-17 are in git
history (`git show e5fc8b09:REAL_PROJECT_COMPAT.md`).

## Corpora

No third-party source is vendored. Every corpus lives under the gitignored
`.local-projects/` (except `unnamed`, which is a sibling checkout), and every
`pnpm run real:<name>` script passes `--allowMissing`, so an absent corpus is
reported as skipped — **not measured**, never as a pass.

| Corpus | Source | Obtain | Target | Role |
| --- | --- | --- | --- | --- |
| **ky** | sindresorhus/ky 2.0.2, `3419113` | manual clone | upstream `tsconfig.json` | **gate**: `pnpm run real:ky:test` asserts exact 0/0 |
| **unnamed** | local Next.js App Router app, `../../nextjs/unnamed` | local only | upstream `tsconfig.json` | **gate**: `pnpm run real:unnamed:test` asserts a count ceiling (a ratchet that only moves down) and that `tsc` still reports 0 |
| **ofetch** | unjs/ofetch, `1dbc37f` | manual clone | upstream `tsconfig.json` | near false-positive corpus (`tsc` reports one removed-option diagnostic) |
| **zod** | colinhacks/zod, `912f0f5` | manual clone | upstream `tsconfig.json` | compatibility target and the main performance corpus |
| **zustand** | pmndrs/zustand v5.0.15, `2115efb` | `pnpm run real:zustand:provision` | upstream root `tsconfig.json`, unmodified | false-positive corpus; the only one measured exactly as it ships |
| **tanstack-query** | TanStack/query, `cdbe8cb` | `pnpm run real:tanstack-query:provision` | aggregate `tsconfig.surge.json` | provisional, not a gate |
| **ts-pattern** | gvergnaud/ts-pattern 5.9.0, `c92ca43` | `pnpm run real:ts-pattern:provision` | aggregate `tsconfig.surge.json` | provisional, not a gate |
| **drizzle-orm** | drizzle-team/drizzle-orm 0.45.3, `b786252` | `pnpm run real:drizzle-orm:provision` | aggregate `drizzle-orm/tsconfig.surge.json` | provisional, not a gate |
| **trpc** | trpc/trpc, `dfbafa8` | manual clone | upstream `tsconfig.json` | measured workload, **never** a parity claim |

Notes on the corpora:

- **Aggregate targets.** tanstack-query, ts-pattern and drizzle-orm have no
  upstream tsconfig that TypeScript 7 can check as-is (root configs cover only
  config files, or inherit the removed `baseUrl` / `downlevelIteration`, which
  the oracle answers with `TS5102` and checks nothing). The aggregates are kept
  in [scripts/real-projects/targets/](scripts/real-projects/targets/) and copied
  into the corpus by the provisioning script. drizzle-orm's is written into the
  *package* directory because `paths` and `typeRoots` resolve relative to the
  tsconfig that declares them.
- **tanstack-query exclusions.** `react-query-devtools` pulls Solid source into
  the program, whose JSX namespace collides with React's under the aggregate's
  `jsx: react-jsx` (the oracle itself reports hundreds of diagnostics), so it is
  left out; it is still measurable on its own tsconfig.
- **drizzle-orm and ts-pattern are not clean-oracle corpora.** TypeScript 7.0.2
  reports diagnostics of its own there, against assertions the libraries wrote
  for older TypeScript (`@ts-expect-error` pairs, `TS2344` on 5.9-era tests).
- **trpc** is a pnpm monorepo whose examples carry unresolved workspace
  imports; `tsc` itself reports over a thousand diagnostics, and the divergence
  is two-sided. Use it to measure, not to claim parity.

## Running a corpus

```bash
pnpm run real:<name>
pnpm run oracle:compare -- --project .local-projects/<name>/tsconfig.json --maxDiagnostics 100000
```

`real:<name>` runs [scripts/real-projects/measure-project.ts](scripts/real-projects/measure-project.ts)
and writes `.bench/real-projects/<name>/` (`measurement.md`,
`compat-report.json`, `oracle-compare.json`, `timings.txt`). For an aggregate
target pass its `tsconfig.surge.json` to `--project` instead.

When recording a number in CURRENT_STATUS.md, follow the
[documentation rules](CLAUDE.md#documentation-rules): measure from a clean
worktree at the cited commit (`SURGE_TS_BIN` pointing at its release binary),
one corpus at a time, and record the corpus checkout alongside the count.

## Local workflow

- Do not commit third-party project source.
- Put disposable real-project experiments under `.local-projects/`.
- Keep the root TypeScript version pinned intentionally; changing it shifts
  oracle output.

```bash
cargo run -p surge-ts-cli -- --project .local-projects/<project>/tsconfig.json --compatReport --maxDiagnostics 200
pnpm run oracle:compare -- --project .local-projects/<project>/tsconfig.json --maxDiagnostics 200
pnpm run oracle:compare -- --file examples/basic.ts
pnpm run oracle:compare -- --file examples/basic.ts --ignoreConfig
```

Use `--project` for `tsconfig.json`-based projects and `--file` for single
source files; passing a `.ts` file to `--project` is rejected so TypeScript
does not misread it as a config input.

## What the report tells you

The compatibility report (`--compatReport`) is raw measurement. It counts the
observed surface — parser errors, unsupported module syntax, non-relative
package and side-effect imports, missing global/lib symbols, plain type
mismatches — without making semantic diagnosis claims, and it does not predict
whether a project is expected to pass.

The oracle comparison is raw measurement too. It gates on diagnostic code-count
and file/code/line; message text and spans are reported but non-gating unless
`--strictMessages` / `--strictSpans` are passed. Diagnostic codes and messages
are catalog-driven in `surge-ts-diagnostics`, so a catalog update can move
oracle output without any checker change.

A green corpus is parity on that corpus at that checkout. It is not a
compatibility claim about the library in general, nor about TypeScript.
