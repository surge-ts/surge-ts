# interface-qualified-heritage-basic

It pins the qualified heritage clause (`interface X extends NS.Member`,
`class C extends NS.Base`) across local, `declare`, global-merged, and
namespace-imported namespaces, plus generic and nested-namespace bases.

The 8 intentional errors in `src/errors.ts` pin inherited-member typing
(TS2322) and closedness (TS2339); everything in `src/index.ts` must stay
diagnostic-free.

## Status: passing, but not registered as an oracle preset

Measured against the pinned oracle (tsc 7.0.2):

| Date | Binary | Diagnostics | Result |
| --- | --- | ---: | --- |
| 2026-07-29 | surge at `b38d381` | 17 | 13 false positives, 4 of tsc's 8 missed |
| **2026-09-01** | **surge at `37dfb3a`** | **8** | **exact 8/8 — code-count, file/code/line, and message text all match** |

The shape the checker could not support has since landed. This fixture is still
**not registered** in `scripts/oracle/compare-tsc.ts`; registering it is open
follow-up work, and doing so would add it to the gated sweep. Verify with:

```bash
pnpm run oracle:compare -- --project tests/compat-projects/interface-qualified-heritage-basic/tsconfig.json --maxDiagnostics 200
```

## Historical: why it was rejected three times

Before it landed, a parse-side fix reached exact parity here three times and was
reverted each time: newly resolved bases more than doubled the degraded
interface resolutions, which are uncacheable by invariant
([docs/PERFORMANCE_INVARIANTS.md](../../../docs/PERFORMANCE_INVARIANTS.md)), and
zod regressed by +59–77% wall time. The measurements are in
`git show e5fc8b09:tests/compat-projects/interface-qualified-heritage-basic/README.md`.

Neighboring behavior this fixture deliberately does **not** cover: a namespace
member reached through a namespace import with three or more segments
(`N.NS.Member`), which resolves correctly in type position since the
2026-07-29 namespace-import alias-table fix (gated by
`namespace-import-qualified-member-basic`).
