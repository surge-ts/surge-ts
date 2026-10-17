# Strict Drift Inventory

Inventory of the non-gating message-text and span/column drift in the oracle
preset sweep. **As of 2026-09-27 (`6c1dfcf5`) 77 of 919 registered
presets drift** — 65 on message text, 15 on column, 3 on both. None
of them fails the normal gate for drift; six other presets fail the normal gate
outright and are listed in [CURRENT_STATUS.md § Gates](CURRENT_STATUS.md#gates).
Earlier snapshots and the passes that closed each drift class were removed on
2026-10-17; `git show e5fc8b09:STRICT_DRIFT_INVENTORY.md` has them.

Drift, when it exists, is confined to the column and to the message text at an
already-correct `(file, code, line)`: every entry recorded here still matched
**code-count and file/code/line** under the normal gate. Drift never implies a
missing, extra, mis-filed, or mis-lined diagnostic.

## Current snapshot (2026-09-27)

- Commit: `6c1dfcf5` (release CLI built in a detached worktree at that commit
  and passed to the sweep with `SURGE_TS_BIN`; the primary working tree had no
  fixture edits).
- TypeScript oracle: 7.0.2 (pinned)
- Scope: all **919** registered oracle presets, `--maxDiagnostics 200`

| Run | Command flags | Result |
| --- | --- | --- |
| Normal gate | (none) | **913 PASS / 6 FAIL** |
| Strict messages | `--strictMessages` | **848 PASS / 71 FAIL** |
| Strict spans | `--strictSpans` | **898 PASS / 21 FAIL** |
| Both | `--strictMessages --strictSpans` | **836 PASS / 83 FAIL** |

The strict counts include the six normal-gate failures. The drifting presets
below match `tsc` at code-count and file/code/line; the delta is the message
text (or column) at that location. Per-location triage has not been redone for
this snapshot; the classes recorded at 2026-09-22 (an alias name where tsc
prints the expansion or the reverse, a fresh literal left unwidened, a member
degraded to `any`/`unknown`, method-versus-property rendering) are the ones to
check first.

**Message-text drift (65 presets):** `arithmetic-operand-rules-basic`, `array-literal-omitted-elements`, `arrow-unit-return-widening-basic`, `assignment-reduction-rules`, `binding-element-default-basic`, `class-heritage-non-constructor-basic`, `conditional-any-check-type-basic`, `conditional-expression-mismatch-anchor-basic`, `discriminant-comparable-domain-basic`, `discriminant-exhaustion-never-basic`, `discriminated-literal-report-basic`, `dotted-namespace-value-member-basic`, `exact-optional-property-types-basic`, `expando-function-member-basic`, `falsy-narrowing-filter-predicate-basic`, `filter-inferred-predicate-basic`, `function-literal-alias-return-inference-basic`, `function-namespace-merge-members`, `generator-grammar-and-iteration-checks`, `generator-return-type-argument`, `generic-reference-missing-type-arguments`, `global-this-member-and-inference-basic`, `index-signature-property-constraint-basic`, `index-slot-narrowing-unchecked-basic`, `indexed-access-index-kinds-basic`, `instantiation-expression-applicability`, `javascript-jsdoc-and-commonjs`, `jsx-attributes-relation-basic`, `jsx-children-attribute-basic`, `jsx-intrinsic-attributes-constituent-basic`, `jsx-intrinsic-attributes-primitive-props-basic`, `jsx-umd-factory-namespace-basic`, `keyof-non-public-members`, `keyof-union-and-mapped-distribution-basic`, `lazy-export-nested-reference-basic`, `logical-or-empty-fallback-basic`, `mapped-type-member-in-body-basic`, `member-write-missing-property-basic`, `member-write-values-checked`, `namespace-merged-member-value`, `namespace-values-and-generators`, `object-literal-normalization-basic`, `object-to-array-missing-members-basic`, `optional-discriminant-narrowing-basic`, `optional-parameter-relation-basic`, `optional-property-source-index-signature`, `recursive-alias-deferred-reference-basic`, `recursive-mapped-alias-member-basic`, `relation-rules-basic`, `relative-directory-specifier`, `rest-tuple-literal-context-basic`, `satisfies-argument-elaboration`, `script-global-values-across-files-basic`, `signature-parameter-typeof-scope-basic`, `template-literal-pattern-basic`, `tsx-jsx-basic`, `tuple-literal-length-anchor-basic`, `tuple-optional-element-flags-basic`, `tuple-rest-target-positions-basic`, `tuple-target-structural-source-basic`, `type-only-namespace-export-type-query-basic`, `type-variable-narrowing-basic`, `umd-global-namespace-type-reference-basic`, `unannotated-body-return-checked-basic`, `weak-type-relations`.

**Column drift (15 presets):** `block-arrow-return-basic`, `generator-yield-types-nonstrict`, `iife-contextual-parameters`, `invalid-write-target-basic`, `invalid-write-target-recovery-basic`, `javascript-jsdoc-and-commonjs`, `merged-interface-type-parameter-defaults`, `namespace-values-and-generators`, `nullish-right-operand-context-only`, `override-and-modifier-order`, `parser-classified-diagnostics-basic`, `property-initialization-literal-names-basic`, `string-slice-arity`, `tsc-regular-expressions`, `weak-type-relations`.

### Commands

```bash
pnpm run oracle:sweep -- --all --maxDiagnostics 200
```

```bash
pnpm run oracle:sweep -- --all --maxDiagnostics 200 --strictMessages
```

```bash
pnpm run oracle:sweep -- --all --maxDiagnostics 200 --strictSpans
```

```bash
pnpm run oracle:sweep -- --all --maxDiagnostics 200 --strictMessages --strictSpans
```

The strict runs exit non-zero by design when drift exists. Pass
`--strictMessages --strictSpans` as two separate arguments; the sweep rejects
them quoted as one.

**Keep the build worktree alive until the measurements are done.** The
generated default-lib subset is resolved through `env!("CARGO_MANIFEST_DIR")`
(`crates/surge-ts-checker/src/default_lib/loader.rs`), so a binary built in a
throwaway worktree stops finding `generated-libs/` the moment that worktree is
removed — even if the binary itself was copied elsewhere first. The failure is
silent apart from a `unknown lib 'es2024.full'` warning on stderr, and it
surfaces as a flood of `TS2304 Cannot find name 'Promise'` on any preset whose
tsconfig does not pin `lib`. Presets that resolve a physical `lib*.d.ts` set are
unaffected, so the damage looks target-specific rather than environmental. Note
that the pinned oracle (`typescript@7.0.2`) ships **no** `lib*.d.ts` files at
all, which is why the generated fallback carries these fixtures.
