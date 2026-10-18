#!/usr/bin/env tsx

import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, statSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export type Source = 'typescript' | 'surge-ts';

export type NormalizedDiagnostic = {
  source: Source;
  code: string;
  fileName: string;
  line?: number;
  column?: number;
  message?: string;
};

export type DiagnosticFingerprint = {
  fileName: string;
  code: string;
  line: number | null;
  column: number | null;
  message: string | null;
};

export type CountBucket = {
  key: string;
  typescript: number;
  surgeTs: number;
};

export type CountEntry = {
  key: string;
  count: number;
};

export type ModuleExportCountEntry = {
  moduleSpecifier: string;
  exportName: string;
  count: number;
};

export type DiagnosticFingerprintCountEntry = DiagnosticFingerprint & {
  count: number;
};

export type DiagnosticTotals = {
  total: number;
  byCode: CountEntry[];
  byFileCode: CountEntry[];
  byFileCodeLine: CountEntry[];
};

export type MessageMismatch = {
  fileName: string;
  code: string;
  line: number | null;
  column: number | null;
  typescript: string;
  surgeTs: string;
};

export type MessageParity = {
  comparedLocations: number;
  matches: number;
  mismatches: MessageMismatch[];
};

export type ComparisonResult = {
  mode: 'project' | 'file';
  project: string | null;
  file: string | null;
  ignoreConfig?: boolean;
  surgeTsOptions?: {
    stubExternalModules?: boolean;
    rustJobs?: number;
  };
  warnings?: string[];
  tooling: {
    typescriptVersion: string;
    typescriptCommand: string;
    surgeTsCommand: string;
    surgeTsJobs?: number;
  };
  typescript: DiagnosticTotals;
  surgeTs: DiagnosticTotals;
  matches: {
    byCode: CountBucket[];
    onlyTypeScript: CountBucket[];
    onlySurgeTs: CountBucket[];
    byFileCode: CountBucket[];
    onlyTypeScriptFileCode: CountBucket[];
    onlySurgeTsFileCode: CountBucket[];
    byFileCodeLine: CountBucket[];
    onlyTypeScriptFileCodeLine: CountBucket[];
    onlySurgeTsFileCodeLine: CountBucket[];
  };
  messageParity: MessageParity;
  summary: {
    byCodeMatch: boolean;
    byFileCodeMatch: boolean;
    byFileCodeLineMatch: boolean | null;
    messageMatch: boolean | null;
  };
  details?: {
    onlySurgeTs?: {
      rawDiagnosticFingerprints?: DiagnosticFingerprintCountEntry[];
      rawTs2305ModuleExports?: ModuleExportCountEntry[];
      rawTs2307ModuleSpecifiers?: CountEntry[];
      rawTs2304Identifiers?: CountEntry[];
    };
    onlyTypeScript?: {
      rawDiagnosticFingerprints?: DiagnosticFingerprintCountEntry[];
    };
  };
};

export type ParsedArgs = {
  projectInput?: string;
  fileInput?: string;
  json: boolean;
  failOnMismatch: boolean;
  strictMessages: boolean;
  maxDiagnostics?: number;
  ignoreConfig?: boolean;
  stubExternalModules?: boolean;
  rustJobs?: number;
};

export type OracleMode =
  | {
      kind: 'project';
      project: string;
      resolvedTsconfig: string;
      ignoreConfig?: boolean;
      stubExternalModules?: boolean;
      rustJobs?: number;
    }
  | {
      kind: 'file';
      file: string;
      resolvedFile: string;
      ignoreConfig?: boolean;
      stubExternalModules?: boolean;
    };

export type RunResult = {
  exitCode: number | null;
  stdout: string;
  stderr: string;
};

const scriptPath = fileURLToPath(import.meta.url);
const scriptDir = path.dirname(scriptPath);
const workspaceRoot = path.resolve(scriptDir, '../..');
const packageManagerCache = process.env.npm_config_cache ?? path.join(os.tmpdir(), 'npm-cache');
// The oracle reference is TypeScript 7.0 (the native compiler, pinned as the
// canonical `typescript` package). Set SURGE_ORACLE_TSC=6 to compare against
// the legacy JS compiler (pinned as the `typescript-6` alias, kept as a
// benchmark reference). Both emit identical
// `file(line,col): error TSxxxx: message` lines, so the parser is shared.
// Each package exposes a `tsc` bin and only one can own `.bin/tsc`, so both are
// invoked through their resolved package bin path rather than `pnpm exec tsc`.
const oracleTypeScript = process.env.SURGE_ORACLE_TSC === '6' ? 'typescript-6' : 'typescript';
const oracleTscBinPath = path.join(workspaceRoot, 'node_modules', oracleTypeScript, 'bin', 'tsc');
const pinnedTypeScriptVersion = readPinnedTypeScriptVersion();
const subprocessMaxBuffer = 50 * 1024 * 1024;

const surgeBinExt = process.platform === 'win32' ? '.exe' : '';
const defaultSurgeBin = path.join(workspaceRoot, 'target', 'debug', `surge${surgeBinExt}`);

let resolvedSurgeBin: string | null = null;

// Resolve the surge-ts CLI binary once per process. Previously every comparison
// shelled out to `cargo run`, which re-runs cargo's freshness check on each
// invocation (multiplied across the sweep's child processes). Building once and
// then executing the binary directly keeps the same freshness guarantee while
// dropping the per-fixture cargo overhead. SURGE_TS_BIN points at a prebuilt
// binary; SURGE_TS_SKIP_BUILD=1 skips the build (set by callers that already built).
export function resolveSurgeBin(): string {
  if (resolvedSurgeBin) {
    return resolvedSurgeBin;
  }

  const override = process.env.SURGE_TS_BIN;
  if (override) {
    if (!existsSync(override)) {
      throw new Error(`SURGE_TS_BIN points to a missing binary: ${override}`);
    }
    resolvedSurgeBin = override;
    return override;
  }

  if (process.env.SURGE_TS_SKIP_BUILD !== '1') {
    const build = spawnSync('cargo', ['build', '-q', '-p', 'surge-ts-cli'], {
      cwd: workspaceRoot,
      encoding: 'utf8',
      maxBuffer: subprocessMaxBuffer,
    });
    if (build.error) {
      throw new Error(`failed to build surge-ts-cli: ${build.error.message}`);
    }
    if (build.status !== 0) {
      throw new Error(`failed to build surge-ts-cli:\n${build.stderr ?? ''}`);
    }
  }

  if (!existsSync(defaultSurgeBin)) {
    throw new Error(
      `surge-ts-cli binary not found at ${defaultSurgeBin}; run \`cargo build -p surge-ts-cli\``,
    );
  }
  resolvedSurgeBin = defaultSurgeBin;
  return defaultSurgeBin;
}

export const fixturePresets: Record<string, string> = {
  'declarations-basic': path.join(workspaceRoot, 'tests/compat-projects/declarations-basic/tsconfig.json'),
  'declarations-hardening': path.join(workspaceRoot, 'tests/compat-projects/declarations-hardening/tsconfig.json'),
  'module-export-visibility-hardening': path.join(workspaceRoot, 'tests/compat-projects/module-export-visibility-hardening/tsconfig.json'),
  'declaration-reexports-hardening': path.join(workspaceRoot, 'tests/compat-projects/declaration-reexports-hardening/tsconfig.json'),
  'nested-predicate-enclosing-type-parameter-basic': path.join(workspaceRoot, 'tests/compat-projects/nested-predicate-enclosing-type-parameter-basic/tsconfig.json'),
  'namespace-import-reexport-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-import-reexport-basic/tsconfig.json'),
  'namespace-nested-member-lazy-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-nested-member-lazy-scope-basic/tsconfig.json'),
  'function-type-binding-pattern-param-basic': path.join(workspaceRoot, 'tests/compat-projects/function-type-binding-pattern-param-basic/tsconfig.json'),
  'interface-extends-call-signature-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-extends-call-signature-basic/tsconfig.json'),
  'package-exports-types-hardening': path.join(workspaceRoot, 'tests/compat-projects/package-exports-types-hardening/tsconfig.json'),
  'diagnostics-pack': path.join(workspaceRoot, 'tests/compat-projects/diagnostics-pack/tsconfig.json'),
  'generics-basic': path.join(workspaceRoot, 'tests/compat-projects/generics-basic/tsconfig.json'),
  'relative-module-augmentation-basic': path.join(workspaceRoot, 'tests/compat-projects/relative-module-augmentation-basic/tsconfig.json'),
  'relative-module-augmentation-heritage-basic': path.join(workspaceRoot, 'tests/compat-projects/relative-module-augmentation-heritage-basic/tsconfig.json'),
  'object-spread-any-source-basic': path.join(workspaceRoot, 'tests/compat-projects/object-spread-any-source-basic/tsconfig.json'),
  'relative-js-extension-substitution-basic': path.join(workspaceRoot, 'tests/compat-projects/relative-js-extension-substitution-basic/tsconfig.json'),
  'relative-directory-index-basic': path.join(workspaceRoot, 'tests/compat-projects/relative-directory-index-basic/tsconfig.json'),
  'import-graph-generated-relative-basic': path.join(workspaceRoot, 'tests/compat-projects/import-graph-generated-relative-basic/tsconfig.json'),
  'paths-wildcard-import-graph-basic': path.join(workspaceRoot, 'tests/compat-projects/paths-wildcard-import-graph-basic/tsconfig.json'),
  'dependency-incomplete-declaration-export-fallback': path.join(workspaceRoot, 'tests/compat-projects/dependency-incomplete-declaration-export-fallback/tsconfig.json'),
  'generic-cache-unresolved-argument-diagnostics-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-cache-unresolved-argument-diagnostics-basic/tsconfig.json'),
  'generic-cache-module-source-not-persisted-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-cache-module-source-not-persisted-basic/tsconfig.json'),
  'generic-cache-dependency-instantiation-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-cache-dependency-instantiation-basic/tsconfig.json'),
  'degraded-generic-argument-conditional-provenance-basic': path.join(workspaceRoot, 'tests/compat-projects/degraded-generic-argument-conditional-provenance-basic/tsconfig.json'),
  'disjoint-protected-intersection-basic': path.join(workspaceRoot, 'tests/compat-projects/disjoint-protected-intersection-basic/tsconfig.json'),
  'implicit-returns-any-return-basic': path.join(workspaceRoot, 'tests/compat-projects/implicit-returns-any-return-basic/tsconfig.json'),
  'undecidable-conditional-never-branch-basic': path.join(workspaceRoot, 'tests/compat-projects/undecidable-conditional-never-branch-basic/tsconfig.json'),
  'skip-lib-check-dependency-dts': path.join(workspaceRoot, 'tests/compat-projects/skip-lib-check-dependency-dts/tsconfig.json'),
  'skip-lib-check-local-dts': path.join(workspaceRoot, 'tests/compat-projects/skip-lib-check-local-dts/tsconfig.json'),
  'package-imports': path.join(workspaceRoot, 'tests/compat-projects/package-imports/tsconfig.json'),
  'module-resolution-follows-module-kind': path.join(workspaceRoot, 'tests/compat-projects/module-resolution-follows-module-kind/tsconfig.json'),
  'versioned-types-conditions': path.join(workspaceRoot, 'tests/compat-projects/versioned-types-conditions/tsconfig.json'),
  'resolution-mode-from-emit-format': path.join(workspaceRoot, 'tests/compat-projects/resolution-mode-from-emit-format/tsconfig.json'),
  'reference-types-resolution-mode': path.join(workspaceRoot, 'tests/compat-projects/reference-types-resolution-mode/tsconfig.json'),
  'import-attribute-resolution-mode': path.join(workspaceRoot, 'tests/compat-projects/import-attribute-resolution-mode/tsconfig.json'),
  'package-imports-module-target': path.join(workspaceRoot, 'tests/compat-projects/package-imports-module-target/tsconfig.json'),
  'nested-package-json-subpath': path.join(workspaceRoot, 'tests/compat-projects/nested-package-json-subpath/tsconfig.json'),
  'json-package-exports-target': path.join(workspaceRoot, 'tests/compat-projects/json-package-exports-target/tsconfig.json'),
  'directive-suppresses-import-diagnostics': path.join(workspaceRoot, 'tests/compat-projects/directive-suppresses-import-diagnostics/tsconfig.json'),
  'relative-directory-specifier': path.join(workspaceRoot, 'tests/compat-projects/relative-directory-specifier/tsconfig.json'),
  'lib-replacement-package': path.join(workspaceRoot, 'tests/compat-projects/lib-replacement-package/tsconfig.json'),
  'cjs-extension-directory-fallback': path.join(workspaceRoot, 'tests/compat-projects/cjs-extension-directory-fallback/tsconfig.json'),
  'side-effect-import-resolution-diagnostics': path.join(workspaceRoot, 'tests/compat-projects/side-effect-import-resolution-diagnostics/tsconfig.json'),
  'module-forms': path.join(workspaceRoot, 'tests/compat-projects/module-forms/tsconfig.json'),
  'relative-deep': path.join(workspaceRoot, 'tests/compat-projects/relative-deep/tsconfig.json'),
  'private-types': path.join(workspaceRoot, 'tests/compat-projects/private-types/tsconfig.json'),
  'package-declarations': path.join(workspaceRoot, 'tests/compat-projects/package-declarations/tsconfig.json'),
  'builtin-visibility-project-graph-basic': path.join(workspaceRoot, 'tests/compat-projects/builtin-visibility-project-graph-basic/tsconfig.json'),
  'builtin-visibility-import-graph-basic': path.join(workspaceRoot, 'tests/compat-projects/builtin-visibility-import-graph-basic/tsconfig.json'),
  'builtin-visibility-function-body-basic': path.join(workspaceRoot, 'tests/compat-projects/builtin-visibility-function-body-basic/tsconfig.json'),
  'module-local-functions-basic': path.join(workspaceRoot, 'tests/compat-projects/module-local-functions-basic/tsconfig.json'),
  'function-body-local-visibility-basic': path.join(workspaceRoot, 'tests/compat-projects/function-body-local-visibility-basic/tsconfig.json'),
  'import-graph-dependency-js-not-source': path.join(workspaceRoot, 'tests/compat-projects/import-graph-dependency-js-not-source/tsconfig.json'),
  'parallel-ordering-basic': path.join(workspaceRoot, 'tests/compat-projects/parallel-ordering-basic/tsconfig.json'),
  'tsx-jsx-basic': path.join(workspaceRoot, 'tests/compat-projects/tsx-jsx-basic/tsconfig.json'),
  'tsx-jsx-expression-diagnostics-basic': path.join(workspaceRoot, 'tests/compat-projects/tsx-jsx-expression-diagnostics-basic/tsconfig.json'),
  'tsx-jsx-attributes-basic': path.join(workspaceRoot, 'tests/compat-projects/tsx-jsx-attributes-basic/tsconfig.json'),
  'tsx-generic-angle-regression-basic': path.join(workspaceRoot, 'tests/compat-projects/tsx-generic-angle-regression-basic/tsconfig.json'),
  'jsx-function-component-props-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-function-component-props-basic/tsconfig.json'),
  'jsx-intrinsic-elements-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-intrinsic-elements-basic/tsconfig.json'),
  'jsx-children-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-children-basic/tsconfig.json'),
  'jsx-component-member-tag-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-component-member-tag-basic/tsconfig.json'),
  'jsx-imported-component-props-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-imported-component-props-basic/tsconfig.json'),
  'jsx-dom-physical-lib-prop-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-dom-physical-lib-prop-basic/tsconfig.json'),
  'jsx-unresolved-no-cascade-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-unresolved-no-cascade-basic/tsconfig.json'),
  'jsx-runtime-module-namespace-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-runtime-module-namespace-basic/tsconfig.json'),
  'jsx-imported-alias-props-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-imported-alias-props-basic/tsconfig.json'),
  'jsx-factory-namespace-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-factory-namespace-basic/tsconfig.json'),
  'jsx-namespace-missing-intrinsics-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-namespace-missing-intrinsics-basic/tsconfig.json'),
  'jsx-attributes-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-attributes-relation-basic/tsconfig.json'),
  'jsx-intrinsic-attributes-constituent-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-intrinsic-attributes-constituent-basic/tsconfig.json'),
  'jsx-children-attribute-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-children-attribute-basic/tsconfig.json'),
  'jsx-attribute-contextual-types-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-attribute-contextual-types-basic/tsconfig.json'),
  'jsx-intrinsic-attributes-primitive-props-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-intrinsic-attributes-primitive-props-basic/tsconfig.json'),
  'jsx-children-arity-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-children-arity-basic/tsconfig.json'),
  'jsx-children-arity-unnamed-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-children-arity-unnamed-basic/tsconfig.json'),
  'jsx-type-only-factory-import-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-type-only-factory-import-basic/tsconfig.json'),
  'jsx-implicit-runtime-import-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-implicit-runtime-import-basic/tsconfig.json'),
  'jsx-umd-factory-namespace-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-umd-factory-namespace-basic/tsconfig.json'),
  'auto-types-node-basic': path.join(workspaceRoot, 'tests/compat-projects/auto-types-node-basic/tsconfig.json'),
  'auto-types-disabled-empty-types-basic': path.join(workspaceRoot, 'tests/compat-projects/auto-types-disabled-empty-types-basic/tsconfig.json'),
  'auto-types-narrowed-types-basic': path.join(workspaceRoot, 'tests/compat-projects/auto-types-narrowed-types-basic/tsconfig.json'),
  'auto-types-scoped-basic': path.join(workspaceRoot, 'tests/compat-projects/auto-types-scoped-basic/tsconfig.json'),
  'auto-types-ancestor-visibility-basic': path.join(workspaceRoot, 'tests/compat-projects/auto-types-ancestor-visibility-basic/packages/app/tsconfig.json'),
  'auto-types-nearest-wins-basic': path.join(workspaceRoot, 'tests/compat-projects/auto-types-nearest-wins-basic/packages/app/tsconfig.json'),
  'type-roots-basic': path.join(workspaceRoot, 'tests/compat-projects/type-roots-basic/tsconfig.json'),
  'type-roots-ignore-default-node-modules-basic': path.join(workspaceRoot, 'tests/compat-projects/type-roots-ignore-default-node-modules-basic/tsconfig.json'),
  'type-roots-with-types-filter-basic': path.join(workspaceRoot, 'tests/compat-projects/type-roots-with-types-filter-basic/tsconfig.json'),
  'reference-types-node-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-node-basic/tsconfig.json'),
  'reference-types-scoped-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-scoped-basic/tsconfig.json'),
  'reference-types-recursive-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-recursive-basic/tsconfig.json'),
  'reference-types-missing-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-missing-basic/tsconfig.json'),
  'reference-types-relative-directive-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-relative-directive-basic/tsconfig.json'),
  'dot-relative-specifier-basic': path.join(workspaceRoot, 'tests/compat-projects/dot-relative-specifier-basic/tsconfig.json'),
  'ambient-module-in-types-entry-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-module-in-types-entry-basic/tsconfig.json'),
  'reference-types-dependency-dts-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-dependency-dts-basic/tsconfig.json'),
  'reference-types-missing-dependency-dts-skip-lib-check-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-missing-dependency-dts-skip-lib-check-basic/tsconfig.json'),
  'reference-types-with-type-roots-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-with-type-roots-basic/tsconfig.json'),
  'reference-types-dedupe-order-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-dedupe-order-basic/tsconfig.json'),
  'node-protocol-buffer-basic': path.join(workspaceRoot, 'tests/compat-projects/node-protocol-buffer-basic/tsconfig.json'),
  'node-protocol-fs-basic': path.join(workspaceRoot, 'tests/compat-projects/node-protocol-fs-basic/tsconfig.json'),
  'node-protocol-type-only-basic': path.join(workspaceRoot, 'tests/compat-projects/node-protocol-type-only-basic/tsconfig.json'),
  'node-protocol-namespace-basic': path.join(workspaceRoot, 'tests/compat-projects/node-protocol-namespace-basic/tsconfig.json'),
  'node-protocol-no-node-types-basic': path.join(workspaceRoot, 'tests/compat-projects/node-protocol-no-node-types-basic/tsconfig.json'),
  'node-protocol-reference-types-node-basic': path.join(workspaceRoot, 'tests/compat-projects/node-protocol-reference-types-node-basic/tsconfig.json'),
  'node-protocol-types-empty-explicit-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/node-protocol-types-empty-explicit-reference-basic/tsconfig.json'),
  'node-protocol-side-effect-import-basic': path.join(workspaceRoot, 'tests/compat-projects/node-protocol-side-effect-import-basic/tsconfig.json'),
  'interface-merging-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-merging-basic/tsconfig.json'),
  'interface-merging-across-files-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-merging-across-files-basic/tsconfig.json'),
  'interface-merging-conflict-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-merging-conflict-basic/tsconfig.json'),
  'declare-global-interface-basic': path.join(workspaceRoot, 'tests/compat-projects/declare-global-interface-basic/tsconfig.json'),
  'declare-global-window-physical-lib-basic': path.join(workspaceRoot, 'tests/compat-projects/declare-global-window-physical-lib-basic/tsconfig.json'),
  'module-augmentation-package-interface-basic': path.join(workspaceRoot, 'tests/compat-projects/module-augmentation-package-interface-basic/tsconfig.json'),
  'module-augmentation-add-export-basic': path.join(workspaceRoot, 'tests/compat-projects/module-augmentation-add-export-basic/tsconfig.json'),
  'ambient-module-reopen-merge-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-module-reopen-merge-basic/tsconfig.json'),
  'ambient-namespace-value-merge-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-namespace-value-merge-basic/tsconfig.json'),
  'ambient-global-namespace-value-merge-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-global-namespace-value-merge-basic/tsconfig.json'),
  'umd-global-module-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/umd-global-module-reference-basic/tsconfig.json'),
  'import-type-value-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/import-type-value-reference-basic/tsconfig.json'),
  'unannotated-return-expression-checks-basic': path.join(workspaceRoot, 'tests/compat-projects/unannotated-return-expression-checks-basic/tsconfig.json'),
  'module-augmentation-unresolved-no-cascade': path.join(workspaceRoot, 'tests/compat-projects/module-augmentation-unresolved-no-cascade/tsconfig.json'),
  'interface-method-merge-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-method-merge-basic/tsconfig.json'),
  'class-interface-merge-policy-pinned': path.join(workspaceRoot, 'tests/compat-projects/class-interface-merge-policy-pinned/tsconfig.json'),
  'physical-lib-iterator-for-of-basic': path.join(workspaceRoot, 'tests/compat-projects/physical-lib-iterator-for-of-basic/tsconfig.json'),
  'react19-jsx-function-component-basic': path.join(workspaceRoot, 'tests/compat-projects/react19-jsx-function-component-basic/tsconfig.json'),
  'react19-jsx-generic-component-basic': path.join(workspaceRoot, 'tests/compat-projects/react19-jsx-generic-component-basic/tsconfig.json'),
  'query-generics-observer-basic': path.join(workspaceRoot, 'tests/compat-projects/query-generics-observer-basic/tsconfig.json'),
  'query-generics-options-mapped-basic': path.join(workspaceRoot, 'tests/compat-projects/query-generics-options-mapped-basic/tsconfig.json'),
  'schema-inference-nested-basic': path.join(workspaceRoot, 'tests/compat-projects/schema-inference-nested-basic/tsconfig.json'),
  'schema-inference-recursive-basic': path.join(workspaceRoot, 'tests/compat-projects/schema-inference-recursive-basic/tsconfig.json'),
  'express-augmentation-cycle-basic': path.join(workspaceRoot, 'tests/compat-projects/express-augmentation-cycle-basic/tsconfig.json'),
  'express-augmentation-cycle-collision-pinned': path.join(workspaceRoot, 'tests/compat-projects/express-augmentation-cycle-collision-pinned/tsconfig.json'),
  'router-graph-procedures-basic': path.join(workspaceRoot, 'tests/compat-projects/router-graph-procedures-basic/tsconfig.json'),
  'router-graph-subscription-basic': path.join(workspaceRoot, 'tests/compat-projects/router-graph-subscription-basic/tsconfig.json'),
  'node-decl-callable-namespace-basic': path.join(workspaceRoot, 'tests/compat-projects/node-decl-callable-namespace-basic/tsconfig.json'),
  'node-decl-subpath-cts-mts-basic': path.join(workspaceRoot, 'tests/compat-projects/node-decl-subpath-cts-mts-basic/tsconfig.json'),
  'combined-conditional-mapped-indexed-basic': path.join(workspaceRoot, 'tests/compat-projects/combined-conditional-mapped-indexed-basic/tsconfig.json'),
  'combined-augmentation-generic-registry-basic': path.join(workspaceRoot, 'tests/compat-projects/combined-augmentation-generic-registry-basic/tsconfig.json'),
  'namespace-import-qualified-member-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-import-qualified-member-basic/tsconfig.json'),
  'namespace-member-signature-siblings-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-member-signature-siblings-basic/tsconfig.json'),
  'enum-member-type-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-member-type-basic/tsconfig.json'),
  'object-literal-property-checking-basic': path.join(workspaceRoot, 'tests/compat-projects/object-literal-property-checking-basic/tsconfig.json'),
  'error-typed-import-binding-basic': path.join(workspaceRoot, 'tests/compat-projects/error-typed-import-binding-basic/tsconfig.json'),
  'instanceof-subclass-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/instanceof-subclass-narrowing-basic/tsconfig.json'),
  'global-interface-cross-file-merge-basic': path.join(workspaceRoot, 'tests/compat-projects/global-interface-cross-file-merge-basic/tsconfig.json'),
  'deferred-block-forward-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/deferred-block-forward-reference-basic/tsconfig.json'),
  'instantiated-indexed-access-basic': path.join(workspaceRoot, 'tests/compat-projects/instantiated-indexed-access-basic/tsconfig.json'),
  'unbound-name-resolution-basic': path.join(workspaceRoot, 'tests/compat-projects/unbound-name-resolution-basic/tsconfig.json'),
  'contextual-return-any-collapse-basic': path.join(workspaceRoot, 'tests/compat-projects/contextual-return-any-collapse-basic/tsconfig.json'),
  'literal-intersection-never-basic': path.join(workspaceRoot, 'tests/compat-projects/literal-intersection-never-basic/tsconfig.json'),
  'intersection-degraded-operand-basic': path.join(workspaceRoot, 'tests/compat-projects/intersection-degraded-operand-basic/tsconfig.json'),
  'intersection-error-type-operand-basic': path.join(workspaceRoot, 'tests/compat-projects/intersection-error-type-operand-basic/tsconfig.json'),
  'relation-target-boolean-nullable-basic': path.join(workspaceRoot, 'tests/compat-projects/relation-target-boolean-nullable-basic/tsconfig.json'),
  'ambient-module-sibling-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-module-sibling-scope-basic/tsconfig.json'),
  'optional-property-conditional-undefined-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-property-conditional-undefined-basic/tsconfig.json'),
  'generic-type-predicate-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-type-predicate-narrowing-basic/tsconfig.json'),
  'nominal-enum-display-basic': path.join(workspaceRoot, 'tests/compat-projects/nominal-enum-display-basic/tsconfig.json'),
  'export-equals-default-import-type-basic': path.join(workspaceRoot, 'tests/compat-projects/export-equals-default-import-type-basic/tsconfig.json'),
  'unresolved-import-callback-implicit-any-basic': path.join(workspaceRoot, 'tests/compat-projects/unresolved-import-callback-implicit-any-basic/tsconfig.json'),
  'callable-object-function-members-basic': path.join(workspaceRoot, 'tests/compat-projects/callable-object-function-members-basic/tsconfig.json'),
  'array-filter-type-predicate-basic': path.join(workspaceRoot, 'tests/compat-projects/array-filter-type-predicate-basic/tsconfig.json'),
  'ambient-script-var-global-augmented-type': path.join(workspaceRoot, 'tests/compat-projects/ambient-script-var-global-augmented-type/tsconfig.json'),
  'ambient-global-function-overload-merge': path.join(workspaceRoot, 'tests/compat-projects/ambient-global-function-overload-merge/tsconfig.json'),
  'this-type-predicate-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/this-type-predicate-narrowing-basic/tsconfig.json'),
  'guard-polarity-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/guard-polarity-narrowing-basic/tsconfig.json'),
  'overload-merge-contextual-callback-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-merge-contextual-callback-basic/tsconfig.json'),
  'namespace-callback-parameter-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-callback-parameter-basic/tsconfig.json'),
  'exit-and-alias-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/exit-and-alias-narrowing-basic/tsconfig.json'),
  'generator-missing-return-basic': path.join(workspaceRoot, 'tests/compat-projects/generator-missing-return-basic/tsconfig.json'),
  'assertion-and-nonnullable-basic': path.join(workspaceRoot, 'tests/compat-projects/assertion-and-nonnullable-basic/tsconfig.json'),
  'class-prototype-instanceof-basic': path.join(workspaceRoot, 'tests/compat-projects/class-prototype-instanceof-basic/tsconfig.json'),
  'global-augmentation-merge-base-scope': path.join(workspaceRoot, 'tests/compat-projects/global-augmentation-merge-base-scope/tsconfig.json'),
  'global-augmentation-fragment-heritage-scope': path.join(workspaceRoot, 'tests/compat-projects/global-augmentation-fragment-heritage-scope/tsconfig.json'),
  'global-declaration-ignores-consumer-locals': path.join(workspaceRoot, 'tests/compat-projects/global-declaration-ignores-consumer-locals/tsconfig.json'),
  'global-this-query-heritage-before-install': path.join(workspaceRoot, 'tests/compat-projects/global-this-query-heritage-before-install/tsconfig.json'),
  'tuple-union-destructure-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-union-destructure-basic/tsconfig.json'),
  'intersection-two-union-operands-basic': path.join(workspaceRoot, 'tests/compat-projects/intersection-two-union-operands-basic/tsconfig.json'),
  'void-parameter-arity-basic': path.join(workspaceRoot, 'tests/compat-projects/void-parameter-arity-basic/tsconfig.json'),
  'type-literal-call-overloads-basic': path.join(workspaceRoot, 'tests/compat-projects/type-literal-call-overloads-basic/tsconfig.json'),
  'union-call-signatures-basic': path.join(workspaceRoot, 'tests/compat-projects/union-call-signatures-basic/tsconfig.json'),
  'conditional-expression-subtype-reduction-basic': path.join(workspaceRoot, 'tests/compat-projects/conditional-expression-subtype-reduction-basic/tsconfig.json'),
  'parameter-initializer-scope': path.join(workspaceRoot, 'tests/compat-projects/parameter-initializer-scope/tsconfig.json'),
  'function-name-in-own-signature': path.join(workspaceRoot, 'tests/compat-projects/function-name-in-own-signature/tsconfig.json'),
  'ambient-type-query-forward-reference': path.join(workspaceRoot, 'tests/compat-projects/ambient-type-query-forward-reference/tsconfig.json'),
  'arrow-type-parameters-in-expression-body': path.join(workspaceRoot, 'tests/compat-projects/arrow-type-parameters-in-expression-body/tsconfig.json'),
  'destructuring-pattern-tuple-context': path.join(workspaceRoot, 'tests/compat-projects/destructuring-pattern-tuple-context/tsconfig.json'),
  'export-clause-global-augmentation': path.join(workspaceRoot, 'tests/compat-projects/export-clause-global-augmentation/tsconfig.json'),
  'export-import-alias-of-imported-namespace': path.join(workspaceRoot, 'tests/compat-projects/export-import-alias-of-imported-namespace/tsconfig.json'),
  'export-default-imported-name-over-global': path.join(workspaceRoot, 'tests/compat-projects/export-default-imported-name-over-global/tsconfig.json'),
  'reexport-namespace-beside-value': path.join(workspaceRoot, 'tests/compat-projects/reexport-namespace-beside-value/tsconfig.json'),
  'export-clause-primitive-and-global-names': path.join(workspaceRoot, 'tests/compat-projects/export-clause-primitive-and-global-names/tsconfig.json'),
  'export-default-inside-namespace': path.join(workspaceRoot, 'tests/compat-projects/export-default-inside-namespace/tsconfig.json'),
  'for-of-head-pattern-bindings': path.join(workspaceRoot, 'tests/compat-projects/for-of-head-pattern-bindings/tsconfig.json'),
  'for-of-var-hoisting': path.join(workspaceRoot, 'tests/compat-projects/for-of-var-hoisting/tsconfig.json'),
  'for-var-head-self-reference': path.join(workspaceRoot, 'tests/compat-projects/for-var-head-self-reference/tsconfig.json'),
  'function-type-parameter-shadows-file-type': path.join(workspaceRoot, 'tests/compat-projects/function-type-parameter-shadows-file-type/tsconfig.json'),
  'mapped-type-parameter-self-constraint': path.join(workspaceRoot, 'tests/compat-projects/mapped-type-parameter-self-constraint/tsconfig.json'),
  'merged-interface-type-parameter-names': path.join(workspaceRoot, 'tests/compat-projects/merged-interface-type-parameter-names/tsconfig.json'),
  'script-global-shadowed-by-module-local': path.join(workspaceRoot, 'tests/compat-projects/script-global-shadowed-by-module-local/tsconfig.json'),
  'type-query-hoisted-function': path.join(workspaceRoot, 'tests/compat-projects/type-query-hoisted-function/tsconfig.json'),
  'block-scoped-self-reference': path.join(workspaceRoot, 'tests/compat-projects/block-scoped-self-reference/tsconfig.json'),
  'assignment-target-resolution': path.join(workspaceRoot, 'tests/compat-projects/assignment-target-resolution/tsconfig.json'),
  'named-function-expression-scope': path.join(workspaceRoot, 'tests/compat-projects/named-function-expression-scope/tsconfig.json'),
  'explicit-return-missing-value': path.join(workspaceRoot, 'tests/compat-projects/explicit-return-missing-value/tsconfig.json'),
  'namespace-merged-member-value': path.join(workspaceRoot, 'tests/compat-projects/namespace-merged-member-value/tsconfig.json'),
  'namespace-block-interface-merging-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-block-interface-merging-basic/tsconfig.json'),
  'mixin-static-members-basic': path.join(workspaceRoot, 'tests/compat-projects/mixin-static-members-basic/tsconfig.json'),
  'missing-dom-property-message-basic': path.join(workspaceRoot, 'tests/compat-projects/missing-dom-property-message-basic/tsconfig.json'),
  'export-equals-entity-meanings': path.join(workspaceRoot, 'tests/compat-projects/export-equals-entity-meanings/tsconfig.json'),
  'import-equals-module-namespace-as-type': path.join(workspaceRoot, 'tests/compat-projects/import-equals-module-namespace-as-type/tsconfig.json'),
  'unresolved-import-generic-reference': path.join(workspaceRoot, 'tests/compat-projects/unresolved-import-generic-reference/tsconfig.json'),
  'type-only-import-equals': path.join(workspaceRoot, 'tests/compat-projects/type-only-import-equals/tsconfig.json'),
  'type-only-export-value-use': path.join(workspaceRoot, 'tests/compat-projects/type-only-export-value-use/tsconfig.json'),
  'export-assignment-type-only-alias': path.join(workspaceRoot, 'tests/compat-projects/export-assignment-type-only-alias/tsconfig.json'),
  'import-equals-namespace-alias-as-value': path.join(workspaceRoot, 'tests/compat-projects/import-equals-namespace-alias-as-value/tsconfig.json'),
  'generic-reference-missing-type-arguments': path.join(workspaceRoot, 'tests/compat-projects/generic-reference-missing-type-arguments/tsconfig.json'),
  'script-expando-const-seed': path.join(workspaceRoot, 'tests/compat-projects/script-expando-const-seed/tsconfig.json'),
  'class-heritage-type-argument-count': path.join(workspaceRoot, 'tests/compat-projects/class-heritage-type-argument-count/tsconfig.json'),
  'generic-reference-type-argument-range': path.join(workspaceRoot, 'tests/compat-projects/generic-reference-type-argument-range/tsconfig.json'),
  'type-only-import-enum': path.join(workspaceRoot, 'tests/compat-projects/type-only-import-enum/tsconfig.json'),
  'ambient-module-block-import-type-query': path.join(workspaceRoot, 'tests/compat-projects/ambient-module-block-import-type-query/tsconfig.json'),
  'ambient-module-import-equals-namespace': path.join(workspaceRoot, 'tests/compat-projects/ambient-module-import-equals-namespace/tsconfig.json'),
  'merged-interface-type-parameter-defaults': path.join(workspaceRoot, 'tests/compat-projects/merged-interface-type-parameter-defaults/tsconfig.json'),
  'construct-overload-inferred-constraint': path.join(workspaceRoot, 'tests/compat-projects/construct-overload-inferred-constraint/tsconfig.json'),
  'global-augmentation-entity-alias-types': path.join(workspaceRoot, 'tests/compat-projects/global-augmentation-entity-alias-types/tsconfig.json'),
  'library-reference-outside-generic-signature': path.join(workspaceRoot, 'tests/compat-projects/library-reference-outside-generic-signature/tsconfig.json'),
  'indexed-access-index-kinds-basic': path.join(workspaceRoot, 'tests/compat-projects/indexed-access-index-kinds-basic/tsconfig.json'),
  'conditional-any-check-type-basic': path.join(workspaceRoot, 'tests/compat-projects/conditional-any-check-type-basic/tsconfig.json'),
  'missing-member-prefix-lookup': path.join(workspaceRoot, 'tests/compat-projects/missing-member-prefix-lookup/tsconfig.json'),
  'property-initializer-constructor-locals': path.join(workspaceRoot, 'tests/compat-projects/property-initializer-constructor-locals/tsconfig.json'),
  'inherited-abstract-generic-heritage': path.join(workspaceRoot, 'tests/compat-projects/inherited-abstract-generic-heritage/tsconfig.json'),
  'global-augmentation-class-alias-values': path.join(workspaceRoot, 'tests/compat-projects/global-augmentation-class-alias-values/tsconfig.json'),
  'object-literal-accessor-pair-typing': path.join(workspaceRoot, 'tests/compat-projects/object-literal-accessor-pair-typing/tsconfig.json'),
  'inference-any-candidate-supertype': path.join(workspaceRoot, 'tests/compat-projects/inference-any-candidate-supertype/tsconfig.json'),
  'inference-extends-any-literal-widening': path.join(workspaceRoot, 'tests/compat-projects/inference-extends-any-literal-widening/tsconfig.json'),
  'inference-object-literal-candidate-union': path.join(workspaceRoot, 'tests/compat-projects/inference-object-literal-candidate-union/tsconfig.json'),
  'inference-contravariant-parameter-candidates': path.join(workspaceRoot, 'tests/compat-projects/inference-contravariant-parameter-candidates/tsconfig.json'),
  'inference-fixed-parameter-widening': path.join(workspaceRoot, 'tests/compat-projects/inference-fixed-parameter-widening/tsconfig.json'),
  'inference-rest-tuple-contextual-parameters': path.join(workspaceRoot, 'tests/compat-projects/inference-rest-tuple-contextual-parameters/tsconfig.json'),
  'inference-callback-parameter-fixing': path.join(workspaceRoot, 'tests/compat-projects/inference-callback-parameter-fixing/tsconfig.json'),
  'inference-fixing-after-other-arguments': path.join(workspaceRoot, 'tests/compat-projects/inference-fixing-after-other-arguments/tsconfig.json'),
  'instantiation-expression-basic': path.join(workspaceRoot, 'tests/compat-projects/instantiation-expression-basic/tsconfig.json'),
  'literal-equality-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/literal-equality-narrowing-basic/tsconfig.json'),
  'optional-chain-guard-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-chain-guard-narrowing-basic/tsconfig.json'),
  'promise-like-intersection-basic': path.join(workspaceRoot, 'tests/compat-projects/promise-like-intersection-basic/tsconfig.json'),
  'namespace-merged-function-export-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-merged-function-export-basic/tsconfig.json'),
  'json-module-import-basic': path.join(workspaceRoot, 'tests/compat-projects/json-module-import-basic/tsconfig.json'),
  'json-module-resolution-disabled-basic': path.join(workspaceRoot, 'tests/compat-projects/json-module-resolution-disabled-basic/tsconfig.json'),
  'instanceof-heritage-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/instanceof-heritage-narrowing-basic/tsconfig.json'),
  'type-literal-method-overloads-basic': path.join(workspaceRoot, 'tests/compat-projects/type-literal-method-overloads-basic/tsconfig.json'),
  'symbol-keyed-indexed-access-basic': path.join(workspaceRoot, 'tests/compat-projects/symbol-keyed-indexed-access-basic/tsconfig.json'),
  'element-access-typeof-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/element-access-typeof-narrowing-basic/tsconfig.json'),
  'union-target-sequence-literal-basic': path.join(workspaceRoot, 'tests/compat-projects/union-target-sequence-literal-basic/tsconfig.json'),
  'union-target-object-literal-member-basic': path.join(workspaceRoot, 'tests/compat-projects/union-target-object-literal-member-basic/tsconfig.json'),
  'const-assertion-nested-literal-basic': path.join(workspaceRoot, 'tests/compat-projects/const-assertion-nested-literal-basic/tsconfig.json'),
  'overload-group-type-predicate-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-group-type-predicate-basic/tsconfig.json'),
  'overload-group-generic-parameter-fold-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-group-generic-parameter-fold-basic/tsconfig.json'),
  'unary-arithmetic-coercion-basic': path.join(workspaceRoot, 'tests/compat-projects/unary-arithmetic-coercion-basic/tsconfig.json'),
  'instantiated-annotation-span-basic': path.join(workspaceRoot, 'tests/compat-projects/instantiated-annotation-span-basic/tsconfig.json'),
  'overload-return-selection-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-return-selection-basic/tsconfig.json'),
  'generic-object-candidate-union-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-object-candidate-union-basic/tsconfig.json'),
  'callback-rest-any-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/callback-rest-any-inference-basic/tsconfig.json'),
  'package-reexport-array-index-basic': path.join(workspaceRoot, 'tests/compat-projects/package-reexport-array-index-basic/tsconfig.json'),
  'alias-double-negation-and-basic': path.join(workspaceRoot, 'tests/compat-projects/alias-double-negation-and-basic/tsconfig.json'),
  'predicate-type-argument-from-arguments-basic': path.join(workspaceRoot, 'tests/compat-projects/predicate-type-argument-from-arguments-basic/tsconfig.json'),
  'generic-class-entity-guard-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-class-entity-guard-basic/tsconfig.json'),
  'parameter-property-default-required-basic': path.join(workspaceRoot, 'tests/compat-projects/parameter-property-default-required-basic/tsconfig.json'),
  'reference-types-declaration-sibling-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-declaration-sibling-basic/tsconfig.json'),
  'abstract-member-no-body-basic': path.join(workspaceRoot, 'tests/compat-projects/abstract-member-no-body-basic/tsconfig.json'),
  'string-keyword-indexed-access-basic': path.join(workspaceRoot, 'tests/compat-projects/string-keyword-indexed-access-basic/tsconfig.json'),
  'filter-arrow-predicate-self-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/filter-arrow-predicate-self-reference-basic/tsconfig.json'),
  'property-path-impossible-union-member-basic': path.join(workspaceRoot, 'tests/compat-projects/property-path-impossible-union-member-basic/tsconfig.json'),
  'typeof-guard-unreachable-branch-basic': path.join(workspaceRoot, 'tests/compat-projects/typeof-guard-unreachable-branch-basic/tsconfig.json'),
  'thenable-awaited-index-access-basic': path.join(workspaceRoot, 'tests/compat-projects/thenable-awaited-index-access-basic/tsconfig.json'),
  'predicate-property-path-subject-basic': path.join(workspaceRoot, 'tests/compat-projects/predicate-property-path-subject-basic/tsconfig.json'),
  'immediately-invoked-arrow-parameters-basic': path.join(workspaceRoot, 'tests/compat-projects/immediately-invoked-arrow-parameters-basic/tsconfig.json'),
  'class-static-inheritance-basic': path.join(workspaceRoot, 'tests/compat-projects/class-static-inheritance-basic/tsconfig.json'),
  'declaration-flow-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/declaration-flow-narrowing-basic/tsconfig.json'),
  'grammar-const-not-initialized-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-const-not-initialized-basic/tsconfig.json'),
  'grammar-duplicate-object-literal-property-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-duplicate-object-literal-property-basic/tsconfig.json'),
  'grammar-missing-implementation-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-missing-implementation-basic/tsconfig.json'),
  'grammar-multiple-default-exports-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-multiple-default-exports-basic/tsconfig.json'),
  'implicit-any-member-and-return-basic': path.join(workspaceRoot, 'tests/compat-projects/implicit-any-member-and-return-basic/tsconfig.json'),
  'missing-properties-plural-basic': path.join(workspaceRoot, 'tests/compat-projects/missing-properties-plural-basic/tsconfig.json'),
  'grammar-class-member-hardening': path.join(workspaceRoot, 'tests/compat-projects/grammar-class-member-hardening/tsconfig.json'),
  'grammar-comma-operator-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-comma-operator-basic/tsconfig.json'),
  'grammar-parameter-list-hardening': path.join(workspaceRoot, 'tests/compat-projects/grammar-parameter-list-hardening/tsconfig.json'),
  'grammar-member-modifier-hardening': path.join(workspaceRoot, 'tests/compat-projects/grammar-member-modifier-hardening/tsconfig.json'),
  'shorthand-property-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/shorthand-property-scope-basic/tsconfig.json'),
  'switch-case-comparability-basic': path.join(workspaceRoot, 'tests/compat-projects/switch-case-comparability-basic/tsconfig.json'),
  'abstract-member-implementation-basic': path.join(workspaceRoot, 'tests/compat-projects/abstract-member-implementation-basic/tsconfig.json'),
  'class-implements-interface-basic': path.join(workspaceRoot, 'tests/compat-projects/class-implements-interface-basic/tsconfig.json'),
  'abstract-class-instantiation-basic': path.join(workspaceRoot, 'tests/compat-projects/abstract-class-instantiation-basic/tsconfig.json'),
  'enum-used-before-declaration-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-used-before-declaration-basic/tsconfig.json'),
  'declaration-form-grammar-basic': path.join(workspaceRoot, 'tests/compat-projects/declaration-form-grammar-basic/tsconfig.json'),
  'interface-index-constraint-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-index-constraint-basic/tsconfig.json'),
  'self-reference-and-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/self-reference-and-placement-basic/tsconfig.json'),
  'import-assertion-keyword-basic': path.join(workspaceRoot, 'tests/compat-projects/import-assertion-keyword-basic/tsconfig.json'),
  'call-and-modifier-context-basic': path.join(workspaceRoot, 'tests/compat-projects/call-and-modifier-context-basic/tsconfig.json'),
  'esm-module-assignment-basic': path.join(workspaceRoot, 'tests/compat-projects/esm-module-assignment-basic/tsconfig.json'),
  'accessor-parameter-grammar-basic': path.join(workspaceRoot, 'tests/compat-projects/accessor-parameter-grammar-basic/tsconfig.json'),
  'class-member-modifier-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/class-member-modifier-placement-basic/tsconfig.json'),
  'this-parameter-modifier-grammar-basic': path.join(workspaceRoot, 'tests/compat-projects/this-parameter-modifier-grammar-basic/tsconfig.json'),
  'private-name-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/private-name-placement-basic/tsconfig.json'),
  'arguments-in-initializer-basic': path.join(workspaceRoot, 'tests/compat-projects/arguments-in-initializer-basic/tsconfig.json'),
  'static-block-return-basic': path.join(workspaceRoot, 'tests/compat-projects/static-block-return-basic/tsconfig.json'),
  'for-in-destructuring-target-basic': path.join(workspaceRoot, 'tests/compat-projects/for-in-destructuring-target-basic/tsconfig.json'),
  'optional-chain-write-targets-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-chain-write-targets-basic/tsconfig.json'),
  'for-of-invalid-target-basic': path.join(workspaceRoot, 'tests/compat-projects/for-of-invalid-target-basic/tsconfig.json'),
  'tagged-template-optional-chain-basic': path.join(workspaceRoot, 'tests/compat-projects/tagged-template-optional-chain-basic/tsconfig.json'),
  'super-call-extends-null-basic': path.join(workspaceRoot, 'tests/compat-projects/super-call-extends-null-basic/tsconfig.json'),
  'super-follower-grammar-basic': path.join(workspaceRoot, 'tests/compat-projects/super-follower-grammar-basic/tsconfig.json'),
  'type-predicate-parameter-basic': path.join(workspaceRoot, 'tests/compat-projects/type-predicate-parameter-basic/tsconfig.json'),
  'computed-name-type-parameter-basic': path.join(workspaceRoot, 'tests/compat-projects/computed-name-type-parameter-basic/tsconfig.json'),
  'catch-clause-annotation-basic': path.join(workspaceRoot, 'tests/compat-projects/catch-clause-annotation-basic/tsconfig.json'),
  'default-module-from-target-basic': path.join(workspaceRoot, 'tests/compat-projects/default-module-from-target-basic/tsconfig.json'),
  'static-function-property-names-basic': path.join(workspaceRoot, 'tests/compat-projects/static-function-property-names-basic/tsconfig.json'),
  'variance-annotation-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/variance-annotation-placement-basic/tsconfig.json'),
  'new-non-constructable-basic': path.join(workspaceRoot, 'tests/compat-projects/new-non-constructable-basic/tsconfig.json'),
  'class-extends-base-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/class-extends-base-relation-basic/tsconfig.json'),
  'class-heritage-non-constructor-basic': path.join(workspaceRoot, 'tests/compat-projects/class-heritage-non-constructor-basic/tsconfig.json'),
  'restricted-constructor-access-basic': path.join(workspaceRoot, 'tests/compat-projects/restricted-constructor-access-basic/tsconfig.json'),
  'super-call-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/super-call-placement-basic/tsconfig.json'),
  'property-initializer-constructor-locals-basic': path.join(workspaceRoot, 'tests/compat-projects/property-initializer-constructor-locals-basic/tsconfig.json'),
  'private-name-writes-basic': path.join(workspaceRoot, 'tests/compat-projects/private-name-writes-basic/tsconfig.json'),
  'commonjs-class-and-import-names-basic': path.join(workspaceRoot, 'tests/compat-projects/commonjs-class-and-import-names-basic/tsconfig.json'),
  'builtin-global-name-conflicts-basic': path.join(workspaceRoot, 'tests/compat-projects/builtin-global-name-conflicts-basic/tsconfig.json'),
  'for-declaration-grammar-basic': path.join(workspaceRoot, 'tests/compat-projects/for-declaration-grammar-basic/tsconfig.json'),
  'function-expression-this-basic': path.join(workspaceRoot, 'tests/compat-projects/function-expression-this-basic/tsconfig.json'),
  'invalid-write-target-basic': path.join(workspaceRoot, 'tests/compat-projects/invalid-write-target-basic/tsconfig.json'),
  'invalid-write-target-recovery-basic': path.join(workspaceRoot, 'tests/compat-projects/invalid-write-target-recovery-basic/tsconfig.json'),
  'element-access-argument-basic': path.join(workspaceRoot, 'tests/compat-projects/element-access-argument-basic/tsconfig.json'),
  'import-declaration-modifiers-basic': path.join(workspaceRoot, 'tests/compat-projects/import-declaration-modifiers-basic/tsconfig.json'),
  'object-member-optional-basic': path.join(workspaceRoot, 'tests/compat-projects/object-member-optional-basic/tsconfig.json'),
  'enum-computed-member-name-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-computed-member-name-basic/tsconfig.json'),
  'ambient-generator-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-generator-basic/tsconfig.json'),
  'class-computed-member-names-basic': path.join(workspaceRoot, 'tests/compat-projects/class-computed-member-names-basic/tsconfig.json'),
  'node-esm-relative-extension-basic': path.join(workspaceRoot, 'tests/compat-projects/node-esm-relative-extension-basic/tsconfig.json'),
  'jump-targets-and-labels-basic': path.join(workspaceRoot, 'tests/compat-projects/jump-targets-and-labels-basic/tsconfig.json'),
  'strict-mode-names-basic': path.join(workspaceRoot, 'tests/compat-projects/strict-mode-names-basic/tsconfig.json'),
  'this-container-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/this-container-rules-basic/tsconfig.json'),
  'catch-yield-await-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/catch-yield-await-placement-basic/tsconfig.json'),
  'super-and-this-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/super-and-this-placement-basic/tsconfig.json'),
  'decorated-static-this-basic': path.join(workspaceRoot, 'tests/compat-projects/decorated-static-this-basic/tsconfig.json'),
  'ambient-and-signature-grammar-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-and-signature-grammar-basic/tsconfig.json'),
  'overload-agreement-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-agreement-basic/tsconfig.json'),
  'write-target-kinds-basic': path.join(workspaceRoot, 'tests/compat-projects/write-target-kinds-basic/tsconfig.json'),
  'operand-type-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/operand-type-rules-basic/tsconfig.json'),
  'object-spread-overwrite-basic': path.join(workspaceRoot, 'tests/compat-projects/object-spread-overwrite-basic/tsconfig.json'),
  'merged-declaration-consistency-basic': path.join(workspaceRoot, 'tests/compat-projects/merged-declaration-consistency-basic/tsconfig.json'),
  'export-assignment-conflicts-basic': path.join(workspaceRoot, 'tests/compat-projects/export-assignment-conflicts-basic/tsconfig.json'),
  'parser-classified-modifier-errors-basic': path.join(workspaceRoot, 'tests/compat-projects/parser-classified-modifier-errors-basic/tsconfig.json'),
  'instantiation-expression-access-basic': path.join(workspaceRoot, 'tests/compat-projects/instantiation-expression-access-basic/tsconfig.json'),
  'regexp-flag-conflict-basic': path.join(workspaceRoot, 'tests/compat-projects/regexp-flag-conflict-basic/tsconfig.json'),
  'exact-optional-property-types-basic': path.join(workspaceRoot, 'tests/compat-projects/exact-optional-property-types-basic/tsconfig.json'),
  'lib-reference-index-and-signature-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/lib-reference-index-and-signature-scope-basic/tsconfig.json'),
  'namespace-reexport-augmentation-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-reexport-augmentation-basic/tsconfig.json'),
  'rest-infer-capture-basic': path.join(workspaceRoot, 'tests/compat-projects/rest-infer-capture-basic/tsconfig.json'),
  'annotated-object-method-parameter-basic': path.join(workspaceRoot, 'tests/compat-projects/annotated-object-method-parameter-basic/tsconfig.json'),
  'property-truthiness-discriminant-basic': path.join(workspaceRoot, 'tests/compat-projects/property-truthiness-discriminant-basic/tsconfig.json'),
  'alias-condition-conditional-expression-basic': path.join(workspaceRoot, 'tests/compat-projects/alias-condition-conditional-expression-basic/tsconfig.json'),
  'cross-module-alias-body-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/cross-module-alias-body-scope-basic/tsconfig.json'),
  'typeof-class-without-value-basic': path.join(workspaceRoot, 'tests/compat-projects/typeof-class-without-value-basic/tsconfig.json'),
  'generic-default-arguments-display-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-default-arguments-display-basic/tsconfig.json'),
  'synthetic-default-import-type-members-basic': path.join(workspaceRoot, 'tests/compat-projects/synthetic-default-import-type-members-basic/tsconfig.json'),
  'module-scope-if-divergence-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/module-scope-if-divergence-narrowing-basic/tsconfig.json'),
  'ambient-class-static-inheritance-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-class-static-inheritance-basic/tsconfig.json'),
  'interface-extends-function-alias-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-extends-function-alias-basic/tsconfig.json'),
  'iterable-element-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/iterable-element-inference-basic/tsconfig.json'),
  'array-iteration-protocol-assignability-basic': path.join(workspaceRoot, 'tests/compat-projects/array-iteration-protocol-assignability-basic/tsconfig.json'),
  'spread-argument-element-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/spread-argument-element-inference-basic/tsconfig.json'),
  'delete-operator-operand-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/delete-operator-operand-rules-basic/tsconfig.json'),
  'increment-operand-write-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/increment-operand-write-rules-basic/tsconfig.json'),
  'nullable-receiver-naming-basic': path.join(workspaceRoot, 'tests/compat-projects/nullable-receiver-naming-basic/tsconfig.json'),
  'nullish-operand-basic': path.join(workspaceRoot, 'tests/compat-projects/nullish-operand-basic/tsconfig.json'),
  'evolving-array-basic': path.join(workspaceRoot, 'tests/compat-projects/evolving-array-basic/tsconfig.json'),
  'type-only-export-value-use-basic': path.join(workspaceRoot, 'tests/compat-projects/type-only-export-value-use-basic/tsconfig.json'),
  'non-module-file-import-basic': path.join(workspaceRoot, 'tests/compat-projects/non-module-file-import-basic/tsconfig.json'),
  'export-global-name-basic': path.join(workspaceRoot, 'tests/compat-projects/export-global-name-basic/tsconfig.json'),
  'module-augmentation-target-basic': path.join(workspaceRoot, 'tests/compat-projects/module-augmentation-target-basic/tsconfig.json'),
  'module-augmentation-subpath-resolution-basic': path.join(workspaceRoot, 'tests/compat-projects/module-augmentation-subpath-resolution-basic/tsconfig.json'),
  'relative-ambient-module-name-basic': path.join(workspaceRoot, 'tests/compat-projects/relative-ambient-module-name-basic/tsconfig.json'),
  'enum-merge-and-const-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-merge-and-const-rules-basic/tsconfig.json'),
  'jsx-flag-missing-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-flag-missing-basic/tsconfig.json'),
  'generic-type-argument-range-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-type-argument-range-basic/tsconfig.json'),
  'unreachable-code-basic': path.join(workspaceRoot, 'tests/compat-projects/unreachable-code-basic/tsconfig.json'),
  'merged-declaration-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/merged-declaration-placement-basic/tsconfig.json'),
  'default-export-merge-basic': path.join(workspaceRoot, 'tests/compat-projects/default-export-merge-basic/tsconfig.json'),
  'module-evolving-array-basic': path.join(workspaceRoot, 'tests/compat-projects/module-evolving-array-basic/tsconfig.json'),
  'auto-type-flow-basic': path.join(workspaceRoot, 'tests/compat-projects/auto-type-flow-basic/tsconfig.json'),
  'strict-null-checks-off-basic': path.join(workspaceRoot, 'tests/compat-projects/strict-null-checks-off-basic/tsconfig.json'),
  'unknown-operand-basic': path.join(workspaceRoot, 'tests/compat-projects/unknown-operand-basic/tsconfig.json'),
  'parser-classified-diagnostics-basic': path.join(workspaceRoot, 'tests/compat-projects/parser-classified-diagnostics-basic/tsconfig.json'),
  'object-spread-operand-type-basic': path.join(workspaceRoot, 'tests/compat-projects/object-spread-operand-type-basic/tsconfig.json'),
  'instanceof-left-operand-type-basic': path.join(workspaceRoot, 'tests/compat-projects/instanceof-left-operand-type-basic/tsconfig.json'),
  'heritage-member-compatibility-basic': path.join(workspaceRoot, 'tests/compat-projects/heritage-member-compatibility-basic/tsconfig.json'),
  'interface-extends-member-compatibility-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-extends-member-compatibility-basic/tsconfig.json'),
  'property-initialization-basic': path.join(workspaceRoot, 'tests/compat-projects/property-initialization-basic/tsconfig.json'),
  'var-redeclaration-type-basic': path.join(workspaceRoot, 'tests/compat-projects/var-redeclaration-type-basic/tsconfig.json'),
  'class-used-before-declaration-basic': path.join(workspaceRoot, 'tests/compat-projects/class-used-before-declaration-basic/tsconfig.json'),
  'property-used-before-initialization-basic': path.join(workspaceRoot, 'tests/compat-projects/property-used-before-initialization-basic/tsconfig.json'),
  'enum-forward-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-forward-reference-basic/tsconfig.json'),
  'generic-class-static-inheritance-no-global-merge-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-class-static-inheritance-no-global-merge-basic/tsconfig.json'),
  'namespace-value-member-annotation-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-value-member-annotation-basic/tsconfig.json'),
  'numeric-enum-number-assignability-basic': path.join(workspaceRoot, 'tests/compat-projects/numeric-enum-number-assignability-basic/tsconfig.json'),
  'import-equals-module-namespace-basic': path.join(workspaceRoot, 'tests/compat-projects/import-equals-module-namespace-basic/tsconfig.json'),
  'reexported-namespace-object-members-basic': path.join(workspaceRoot, 'tests/compat-projects/reexported-namespace-object-members-basic/tsconfig.json'),
  'argument-mismatch-first-only-basic': path.join(workspaceRoot, 'tests/compat-projects/argument-mismatch-first-only-basic/tsconfig.json'),
  'unused-destructured-parameters-basic': path.join(workspaceRoot, 'tests/compat-projects/unused-destructured-parameters-basic/tsconfig.json'),
  'annotated-union-written-unknown-initializer-basic': path.join(workspaceRoot, 'tests/compat-projects/annotated-union-written-unknown-initializer-basic/tsconfig.json'),
  'generic-class-constructor-argument-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-class-constructor-argument-inference-basic/tsconfig.json'),
  'generic-callback-argument-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-callback-argument-inference-basic/tsconfig.json'),
  'possibly-undefined-receiver-basic': path.join(workspaceRoot, 'tests/compat-projects/possibly-undefined-receiver-basic/tsconfig.json'),
  'no-unchecked-indexed-access-basic': path.join(workspaceRoot, 'tests/compat-projects/no-unchecked-indexed-access-basic/tsconfig.json'),
  'write-target-types-basic': path.join(workspaceRoot, 'tests/compat-projects/write-target-types-basic/tsconfig.json'),
  'numeric-index-signature-basic': path.join(workspaceRoot, 'tests/compat-projects/numeric-index-signature-basic/tsconfig.json'),
  'ts-extension-import-basic': path.join(workspaceRoot, 'tests/compat-projects/ts-extension-import-basic/tsconfig.json'),
  'spelling-suggestion-basic': path.join(workspaceRoot, 'tests/compat-projects/spelling-suggestion-basic/tsconfig.json'),
  'logical-assignment-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/logical-assignment-narrowing-basic/tsconfig.json'),
  'element-access-guard-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/element-access-guard-narrowing-basic/tsconfig.json'),
  'intersection-self-reference-cycle-basic': path.join(workspaceRoot, 'tests/compat-projects/intersection-self-reference-cycle-basic/tsconfig.json'),
  'optional-value-union-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-value-union-inference-basic/tsconfig.json'),
  'exports-dotted-runtime-target-basic': path.join(workspaceRoot, 'tests/compat-projects/exports-dotted-runtime-target-basic/tsconfig.json'),
  'arrow-argument-parameter-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/arrow-argument-parameter-inference-basic/tsconfig.json'),
  'array-reference-parameter-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/array-reference-parameter-inference-basic/tsconfig.json'),
  'rest-parameter-alias-annotation-basic': path.join(workspaceRoot, 'tests/compat-projects/rest-parameter-alias-annotation-basic/tsconfig.json'),
  'awaited-nullable-union-alias-basic': path.join(workspaceRoot, 'tests/compat-projects/awaited-nullable-union-alias-basic/tsconfig.json'),
  'property-literal-equality-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/property-literal-equality-narrowing-basic/tsconfig.json'),
  'typeof-function-member-generic-call-basic': path.join(workspaceRoot, 'tests/compat-projects/typeof-function-member-generic-call-basic/tsconfig.json'),
  'method-type-parameter-default-basic': path.join(workspaceRoot, 'tests/compat-projects/method-type-parameter-default-basic/tsconfig.json'),
  'callback-return-type-parameter-basic': path.join(workspaceRoot, 'tests/compat-projects/callback-return-type-parameter-basic/tsconfig.json'),
  'mapped-type-optionality-modifier-basic': path.join(workspaceRoot, 'tests/compat-projects/mapped-type-optionality-modifier-basic/tsconfig.json'),
  'declared-literal-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/declared-literal-inference-basic/tsconfig.json'),
  'callable-intersection-brand-basic': path.join(workspaceRoot, 'tests/compat-projects/callable-intersection-brand-basic/tsconfig.json'),
  'ambient-namespace-member-contextual-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-namespace-member-contextual-basic/tsconfig.json'),
  'missing-return-diagnostic-selection-basic': path.join(workspaceRoot, 'tests/compat-projects/missing-return-diagnostic-selection-basic/tsconfig.json'),
  'angle-bracket-assertion-basic': path.join(workspaceRoot, 'tests/compat-projects/angle-bracket-assertion-basic/tsconfig.json'),
  'assertion-literal-freshness-basic': path.join(workspaceRoot, 'tests/compat-projects/assertion-literal-freshness-basic/tsconfig.json'),
  'const-object-literal-member-widening-basic': path.join(workspaceRoot, 'tests/compat-projects/const-object-literal-member-widening-basic/tsconfig.json'),
  'assignment-target-anchor-basic': path.join(workspaceRoot, 'tests/compat-projects/assignment-target-anchor-basic/tsconfig.json'),
  'conditional-expression-mismatch-anchor-basic': path.join(workspaceRoot, 'tests/compat-projects/conditional-expression-mismatch-anchor-basic/tsconfig.json'),
  'arrow-return-elaboration-basic': path.join(workspaceRoot, 'tests/compat-projects/arrow-return-elaboration-basic/tsconfig.json'),
  'arrow-unit-return-widening-basic': path.join(workspaceRoot, 'tests/compat-projects/arrow-unit-return-widening-basic/tsconfig.json'),
  'member-write-missing-property-basic': path.join(workspaceRoot, 'tests/compat-projects/member-write-missing-property-basic/tsconfig.json'),
  'this-write-missing-property-basic': path.join(workspaceRoot, 'tests/compat-projects/this-write-missing-property-basic/tsconfig.json'),
  'this-write-target-anchor-basic': path.join(workspaceRoot, 'tests/compat-projects/this-write-target-anchor-basic/tsconfig.json'),
  'this-parameter-typing-basic': path.join(workspaceRoot, 'tests/compat-projects/this-parameter-typing-basic/tsconfig.json'),
  'class-field-initializer-type-basic': path.join(workspaceRoot, 'tests/compat-projects/class-field-initializer-type-basic/tsconfig.json'),
  'tuple-index-out-of-bounds-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-index-out-of-bounds-basic/tsconfig.json'),
  'destructuring-assignment-basic': path.join(workspaceRoot, 'tests/compat-projects/destructuring-assignment-basic/tsconfig.json'),
  'or-false-branch-reference-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/or-false-branch-reference-narrowing-basic/tsconfig.json'),
  'optional-chain-discriminant-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-chain-discriminant-narrowing-basic/tsconfig.json'),
  'keyof-union-and-mapped-distribution-basic': path.join(workspaceRoot, 'tests/compat-projects/keyof-union-and-mapped-distribution-basic/tsconfig.json'),
  'uncalled-function-condition-basic': path.join(workspaceRoot, 'tests/compat-projects/uncalled-function-condition-basic/tsconfig.json'),
  'overload-argument-mismatch-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-argument-mismatch-basic/tsconfig.json'),
  'loop-assignment-join-basic': path.join(workspaceRoot, 'tests/compat-projects/loop-assignment-join-basic/tsconfig.json'),
  'non-exhaustive-switch-missing-return-basic': path.join(workspaceRoot, 'tests/compat-projects/non-exhaustive-switch-missing-return-basic/tsconfig.json'),
  'readonly-array-element-write-basic': path.join(workspaceRoot, 'tests/compat-projects/readonly-array-element-write-basic/tsconfig.json'),
  'named-constraint-primitive-argument-basic': path.join(workspaceRoot, 'tests/compat-projects/named-constraint-primitive-argument-basic/tsconfig.json'),
  'unannotated-method-return-type-basic': path.join(workspaceRoot, 'tests/compat-projects/unannotated-method-return-type-basic/tsconfig.json'),
  'override-modifier-without-base-member-basic': path.join(workspaceRoot, 'tests/compat-projects/override-modifier-without-base-member-basic/tsconfig.json'),
  'import-binding-assignment-basic': path.join(workspaceRoot, 'tests/compat-projects/import-binding-assignment-basic/tsconfig.json'),
  'import-unexported-local-basic': path.join(workspaceRoot, 'tests/compat-projects/import-unexported-local-basic/tsconfig.json'),
  'modifier-order-basic': path.join(workspaceRoot, 'tests/compat-projects/modifier-order-basic/tsconfig.json'),
  'arithmetic-operand-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/arithmetic-operand-rules-basic/tsconfig.json'),
  'equality-reference-and-nan-basic': path.join(workspaceRoot, 'tests/compat-projects/equality-reference-and-nan-basic/tsconfig.json'),
  'symbol-operand-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/symbol-operand-rules-basic/tsconfig.json'),
  'in-operator-operand-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/in-operator-operand-rules-basic/tsconfig.json'),
  'branded-primitive-assignability-basic': path.join(workspaceRoot, 'tests/compat-projects/branded-primitive-assignability-basic/tsconfig.json'),
  'unary-operand-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/unary-operand-rules-basic/tsconfig.json'),
  'template-expression-type-basic': path.join(workspaceRoot, 'tests/compat-projects/template-expression-type-basic/tsconfig.json'),
  'constructor-overload-group-basic': path.join(workspaceRoot, 'tests/compat-projects/constructor-overload-group-basic/tsconfig.json'),
  'void-expression-type-basic': path.join(workspaceRoot, 'tests/compat-projects/void-expression-type-basic/tsconfig.json'),
  'spread-iterability-basic': path.join(workspaceRoot, 'tests/compat-projects/spread-iterability-basic/tsconfig.json'),
  'element-access-missing-key-basic': path.join(workspaceRoot, 'tests/compat-projects/element-access-missing-key-basic/tsconfig.json'),
  'array-pattern-iterability-basic': path.join(workspaceRoot, 'tests/compat-projects/array-pattern-iterability-basic/tsconfig.json'),
  'for-of-non-null-source-basic': path.join(workspaceRoot, 'tests/compat-projects/for-of-non-null-source-basic/tsconfig.json'),
  'property-spelling-suggestion-basic': path.join(workspaceRoot, 'tests/compat-projects/property-spelling-suggestion-basic/tsconfig.json'),
  'name-spelling-suggestion-basic': path.join(workspaceRoot, 'tests/compat-projects/name-spelling-suggestion-basic/tsconfig.json'),
  'import-member-suggestion-basic': path.join(workspaceRoot, 'tests/compat-projects/import-member-suggestion-basic/tsconfig.json'),
  'module-condition-comparison-basic': path.join(workspaceRoot, 'tests/compat-projects/module-condition-comparison-basic/tsconfig.json'),
  'parser-grammar-codes-basic': path.join(workspaceRoot, 'tests/compat-projects/parser-grammar-codes-basic/tsconfig.json'),
  'async-return-type-promise-basic': path.join(workspaceRoot, 'tests/compat-projects/async-return-type-promise-basic/tsconfig.json'),
  'overload-implementation-compatibility-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-implementation-compatibility-basic/tsconfig.json'),
  'enum-member-constant-values-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-member-constant-values-basic/tsconfig.json'),
  'accessor-declaration-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/accessor-declaration-rules-basic/tsconfig.json'),
  'enum-duplicate-members-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-duplicate-members-basic/tsconfig.json'),
  'relational-comparison-operands-basic': path.join(workspaceRoot, 'tests/compat-projects/relational-comparison-operands-basic/tsconfig.json'),
  'relational-comparable-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/relational-comparable-relation-basic/tsconfig.json'),
  'readonly-array-mutable-target-basic': path.join(workspaceRoot, 'tests/compat-projects/readonly-array-mutable-target-basic/tsconfig.json'),
  'equality-comparable-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/equality-comparable-relation-basic/tsconfig.json'),
  'any-never-target-basic': path.join(workspaceRoot, 'tests/compat-projects/any-never-target-basic/tsconfig.json'),
  'class-accessor-bodies-basic': path.join(workspaceRoot, 'tests/compat-projects/class-accessor-bodies-basic/tsconfig.json'),
  'return-mismatch-anchor-basic': path.join(workspaceRoot, 'tests/compat-projects/return-mismatch-anchor-basic/tsconfig.json'),
  'arrow-expression-body-return-basic': path.join(workspaceRoot, 'tests/compat-projects/arrow-expression-body-return-basic/tsconfig.json'),
  'switch-fallthrough-return-basic': path.join(workspaceRoot, 'tests/compat-projects/switch-fallthrough-return-basic/tsconfig.json'),
  'function-expression-missing-return-basic': path.join(workspaceRoot, 'tests/compat-projects/function-expression-missing-return-basic/tsconfig.json'),
  'object-accessor-setter-body-basic': path.join(workspaceRoot, 'tests/compat-projects/object-accessor-setter-body-basic/tsconfig.json'),
  'class-static-block-body-basic': path.join(workspaceRoot, 'tests/compat-projects/class-static-block-body-basic/tsconfig.json'),
  'computed-accessor-body-basic': path.join(workspaceRoot, 'tests/compat-projects/computed-accessor-body-basic/tsconfig.json'),
  'call-callee-and-arity-arguments-basic': path.join(workspaceRoot, 'tests/compat-projects/call-callee-and-arity-arguments-basic/tsconfig.json'),
  'primitive-assertion-overlap-basic': path.join(workspaceRoot, 'tests/compat-projects/primitive-assertion-overlap-basic/tsconfig.json'),
  'tagged-template-call-basic': path.join(workspaceRoot, 'tests/compat-projects/tagged-template-call-basic/tsconfig.json'),
  'this-before-super-basic': path.join(workspaceRoot, 'tests/compat-projects/this-before-super-basic/tsconfig.json'),
  'member-kind-override-basic': path.join(workspaceRoot, 'tests/compat-projects/member-kind-override-basic/tsconfig.json'),
  'super-member-access-basic': path.join(workspaceRoot, 'tests/compat-projects/super-member-access-basic/tsconfig.json'),
  'never-parameter-literal-argument-basic': path.join(workspaceRoot, 'tests/compat-projects/never-parameter-literal-argument-basic/tsconfig.json'),
  'call-type-argument-count-basic': path.join(workspaceRoot, 'tests/compat-projects/call-type-argument-count-basic/tsconfig.json'),
  'string-indexed-access-type-basic': path.join(workspaceRoot, 'tests/compat-projects/string-indexed-access-type-basic/tsconfig.json'),
  'type-parameter-default-order-basic': path.join(workspaceRoot, 'tests/compat-projects/type-parameter-default-order-basic/tsconfig.json'),
  'circular-type-alias-basic': path.join(workspaceRoot, 'tests/compat-projects/circular-type-alias-basic/tsconfig.json'),
  'no-default-export-import-basic': path.join(workspaceRoot, 'tests/compat-projects/no-default-export-import-basic/tsconfig.json'),
  'import-local-declaration-conflict-basic': path.join(workspaceRoot, 'tests/compat-projects/import-local-declaration-conflict-basic/tsconfig.json'),
  'duplicate-export-declaration-basic': path.join(workspaceRoot, 'tests/compat-projects/duplicate-export-declaration-basic/tsconfig.json'),
  'declaration-file-ambient-grammar': path.join(workspaceRoot, 'tests/compat-projects/declaration-file-ambient-grammar/tsconfig.json'),
  'duplicate-export-specifier-basic': path.join(workspaceRoot, 'tests/compat-projects/duplicate-export-specifier-basic/tsconfig.json'),
  'declaration-merging-values-basic': path.join(workspaceRoot, 'tests/compat-projects/declaration-merging-values-basic/tsconfig.json'),
  'non-literal-index-key-basic': path.join(workspaceRoot, 'tests/compat-projects/non-literal-index-key-basic/tsconfig.json'),
  'numeric-index-receiver-basic': path.join(workspaceRoot, 'tests/compat-projects/numeric-index-receiver-basic/tsconfig.json'),
  'member-accessibility-basic': path.join(workspaceRoot, 'tests/compat-projects/member-accessibility-basic/tsconfig.json'),
  'null-type-basic': path.join(workspaceRoot, 'tests/compat-projects/null-type-basic/tsconfig.json'),
  'ambient-implicit-any': path.join(workspaceRoot, 'tests/compat-projects/ambient-implicit-any/tsconfig.json'),
  'null-type-loose-basic': path.join(workspaceRoot, 'tests/compat-projects/null-type-loose-basic/tsconfig.json'),
  'namespace-body-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-body-basic/tsconfig.json'),
  'relation-rules-basic': path.join(workspaceRoot, 'tests/compat-projects/relation-rules-basic/tsconfig.json'),
  'enum-nominal-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-nominal-basic/tsconfig.json'),
  'loop-assignment-flow-basic': path.join(workspaceRoot, 'tests/compat-projects/loop-assignment-flow-basic/tsconfig.json'),
  'rest-tuple-parameters-basic': path.join(workspaceRoot, 'tests/compat-projects/rest-tuple-parameters-basic/tsconfig.json'),
  'typeof-narrow-subtype-basic': path.join(workspaceRoot, 'tests/compat-projects/typeof-narrow-subtype-basic/tsconfig.json'),
  'discriminated-literal-report-basic': path.join(workspaceRoot, 'tests/compat-projects/discriminated-literal-report-basic/tsconfig.json'),
  'inferred-type-predicate-basic': path.join(workspaceRoot, 'tests/compat-projects/inferred-type-predicate-basic/tsconfig.json'),
  'overload-predicate-callback-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-predicate-callback-basic/tsconfig.json'),
  'typeof-unmatched-tag-basic': path.join(workspaceRoot, 'tests/compat-projects/typeof-unmatched-tag-basic/tsconfig.json'),
  'interface-call-overloads-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-call-overloads-basic/tsconfig.json'),
  'typed-array-cross-assign-basic': path.join(workspaceRoot, 'tests/compat-projects/typed-array-cross-assign-basic/tsconfig.json'),
  'template-literal-pattern-basic': path.join(workspaceRoot, 'tests/compat-projects/template-literal-pattern-basic/tsconfig.json'),
  'literal-element-context-basic': path.join(workspaceRoot, 'tests/compat-projects/literal-element-context-basic/tsconfig.json'),
  'generic-signature-context-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-signature-context-basic/tsconfig.json'),
  'optional-parameter-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-parameter-relation-basic/tsconfig.json'),
  'object-signature-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/object-signature-relation-basic/tsconfig.json'),
  'method-signature-implicit-return-basic': path.join(workspaceRoot, 'tests/compat-projects/method-signature-implicit-return-basic/tsconfig.json'),
  'index-signature-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/index-signature-relation-basic/tsconfig.json'),
  'spread-argument-check-basic': path.join(workspaceRoot, 'tests/compat-projects/spread-argument-check-basic/tsconfig.json'),
  'promise-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/promise-relation-basic/tsconfig.json'),
  'element-read-dependent-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/element-read-dependent-narrowing-basic/tsconfig.json'),
  'unknown-guard-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/unknown-guard-narrowing-basic/tsconfig.json'),
  'comma-expression-value-basic': path.join(workspaceRoot, 'tests/compat-projects/comma-expression-value-basic/tsconfig.json'),
  'property-signature-implicit-any-basic': path.join(workspaceRoot, 'tests/compat-projects/property-signature-implicit-any-basic/tsconfig.json'),
  'primitive-apparent-type-basic': path.join(workspaceRoot, 'tests/compat-projects/primitive-apparent-type-basic/tsconfig.json'),
  'inferable-index-signature-basic': path.join(workspaceRoot, 'tests/compat-projects/inferable-index-signature-basic/tsconfig.json'),
  'logical-operand-value-basic': path.join(workspaceRoot, 'tests/compat-projects/logical-operand-value-basic/tsconfig.json'),
  'instanceof-constructor-value-basic': path.join(workspaceRoot, 'tests/compat-projects/instanceof-constructor-value-basic/tsconfig.json'),
  'binding-element-default-basic': path.join(workspaceRoot, 'tests/compat-projects/binding-element-default-basic/tsconfig.json'),
  'for-header-clauses-basic': path.join(workspaceRoot, 'tests/compat-projects/for-header-clauses-basic/tsconfig.json'),
  'tuple-length-anchor-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-length-anchor-basic/tsconfig.json'),
  'parenthesized-assignment-target-basic': path.join(workspaceRoot, 'tests/compat-projects/parenthesized-assignment-target-basic/tsconfig.json'),
  'object-to-array-missing-members-basic': path.join(workspaceRoot, 'tests/compat-projects/object-to-array-missing-members-basic/tsconfig.json'),
  'callback-argument-signature-basic': path.join(workspaceRoot, 'tests/compat-projects/callback-argument-signature-basic/tsconfig.json'),
  'object-literal-union-head-basic': path.join(workspaceRoot, 'tests/compat-projects/object-literal-union-head-basic/tsconfig.json'),
  'object-literal-normalization-basic': path.join(workspaceRoot, 'tests/compat-projects/object-literal-normalization-basic/tsconfig.json'),
  'optional-chain-containment-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-chain-containment-basic/tsconfig.json'),
  'dependent-object-destructuring-basic': path.join(workspaceRoot, 'tests/compat-projects/dependent-object-destructuring-basic/tsconfig.json'),
  'this-rooted-discriminant-basic': path.join(workspaceRoot, 'tests/compat-projects/this-rooted-discriminant-basic/tsconfig.json'),
  'lib-builtin-member-fallback-basic': path.join(workspaceRoot, 'tests/compat-projects/lib-builtin-member-fallback-basic/tsconfig.json'),
  'mixin-constructor-intersection-basic': path.join(workspaceRoot, 'tests/compat-projects/mixin-constructor-intersection-basic/tsconfig.json'),
  'truthy-falsy-literal-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/truthy-falsy-literal-narrowing-basic/tsconfig.json'),
  'falsy-branch-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/falsy-branch-narrowing-basic/tsconfig.json'),
  'auto-accessor-member-basic': path.join(workspaceRoot, 'tests/compat-projects/auto-accessor-member-basic/tsconfig.json'),
  'constructor-equality-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/constructor-equality-narrowing-basic/tsconfig.json'),
  'union-receiver-missing-property-basic': path.join(workspaceRoot, 'tests/compat-projects/union-receiver-missing-property-basic/tsconfig.json'),
  'discriminant-comparable-domain-basic': path.join(workspaceRoot, 'tests/compat-projects/discriminant-comparable-domain-basic/tsconfig.json'),
  'logical-or-empty-fallback-basic': path.join(workspaceRoot, 'tests/compat-projects/logical-or-empty-fallback-basic/tsconfig.json'),
  'reference-path-discriminant-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-path-discriminant-basic/tsconfig.json'),
  'index-slot-narrowing-unchecked-basic': path.join(workspaceRoot, 'tests/compat-projects/index-slot-narrowing-unchecked-basic/tsconfig.json'),
  'defaulted-parameter-optionality-basic': path.join(workspaceRoot, 'tests/compat-projects/defaulted-parameter-optionality-basic/tsconfig.json'),
  'inferred-type-predicate-declaration-basic': path.join(workspaceRoot, 'tests/compat-projects/inferred-type-predicate-declaration-basic/tsconfig.json'),
  'expando-function-member-basic': path.join(workspaceRoot, 'tests/compat-projects/expando-function-member-basic/tsconfig.json'),
  'module-block-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/module-block-scope-basic/tsconfig.json'),
  'lib-constructor-feature-basic': path.join(workspaceRoot, 'tests/compat-projects/lib-constructor-feature-basic/tsconfig.json'),
  'array-reduce-accumulator-basic': path.join(workspaceRoot, 'tests/compat-projects/array-reduce-accumulator-basic/tsconfig.json'),
  'type-variable-relation-basic': path.join(workspaceRoot, 'tests/compat-projects/type-variable-relation-basic/tsconfig.json'),
  'type-variable-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/type-variable-narrowing-basic/tsconfig.json'),
  'member-write-narrowing-join-basic': path.join(workspaceRoot, 'tests/compat-projects/member-write-narrowing-join-basic/tsconfig.json'),
  'index-signature-property-constraint-basic': path.join(workspaceRoot, 'tests/compat-projects/index-signature-property-constraint-basic/tsconfig.json'),
  'promise-all-tuple-basic': path.join(workspaceRoot, 'tests/compat-projects/promise-all-tuple-basic/tsconfig.json'),
  'type-parameter-default-primitive-context-basic': path.join(workspaceRoot, 'tests/compat-projects/type-parameter-default-primitive-context-basic/tsconfig.json'),
  'try-catch-never-call-basic': path.join(workspaceRoot, 'tests/compat-projects/try-catch-never-call-basic/tsconfig.json'),
  'promise-race-union-basic': path.join(workspaceRoot, 'tests/compat-projects/promise-race-union-basic/tsconfig.json'),
  'interface-method-overload-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-method-overload-basic/tsconfig.json'),
  'destructured-boolean-discriminant-basic': path.join(workspaceRoot, 'tests/compat-projects/destructured-boolean-discriminant-basic/tsconfig.json'),
  'cannot-find-name-variants-basic': path.join(workspaceRoot, 'tests/compat-projects/cannot-find-name-variants-basic/tsconfig.json'),
  'cannot-find-name-lib-basic': path.join(workspaceRoot, 'tests/compat-projects/cannot-find-name-lib-basic/tsconfig.json'),
  'nested-function-body-basic': path.join(workspaceRoot, 'tests/compat-projects/nested-function-body-basic/tsconfig.json'),
  'class-type-parameter-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/class-type-parameter-scope-basic/tsconfig.json'),
  'heritage-await-globalthis-basic': path.join(workspaceRoot, 'tests/compat-projects/heritage-await-globalthis-basic/tsconfig.json'),
  'jsx-missing-intrinsic-elements-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-missing-intrinsic-elements-basic/tsconfig.json'),
  'recursive-alias-and-widening-basic': path.join(workspaceRoot, 'tests/compat-projects/recursive-alias-and-widening-basic/tsconfig.json'),
  'global-this-member-and-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/global-this-member-and-inference-basic/tsconfig.json'),
  'readonly-this-write-basic': path.join(workspaceRoot, 'tests/compat-projects/readonly-this-write-basic/tsconfig.json'),
  'base-constructor-signatures-basic': path.join(workspaceRoot, 'tests/compat-projects/base-constructor-signatures-basic/tsconfig.json'),
  'class-value-heritage-basic': path.join(workspaceRoot, 'tests/compat-projects/class-value-heritage-basic/tsconfig.json'),
  'type-only-namespace-export-type-query-basic': path.join(workspaceRoot, 'tests/compat-projects/type-only-namespace-export-type-query-basic/tsconfig.json'),
  'overloaded-signature-infer-return-basic': path.join(workspaceRoot, 'tests/compat-projects/overloaded-signature-infer-return-basic/tsconfig.json'),
  'discriminant-exhaustion-never-basic': path.join(workspaceRoot, 'tests/compat-projects/discriminant-exhaustion-never-basic/tsconfig.json'),
  'narrowing-unassigned-read-declared-type': path.join(workspaceRoot, 'tests/compat-projects/narrowing-unassigned-read-declared-type/tsconfig.json'),
  'inferred-type-predicates-basic': path.join(workspaceRoot, 'tests/compat-projects/inferred-type-predicates-basic/tsconfig.json'),
  'narrowing-instanceof-construct-signature': path.join(workspaceRoot, 'tests/compat-projects/narrowing-instanceof-construct-signature/tsconfig.json'),
  'narrowing-this-type-predicate': path.join(workspaceRoot, 'tests/compat-projects/narrowing-this-type-predicate/tsconfig.json'),
  'template-literal-constant-evaluation': path.join(workspaceRoot, 'tests/compat-projects/template-literal-constant-evaluation/tsconfig.json'),
  'narrowing-logical-operand-edges': path.join(workspaceRoot, 'tests/compat-projects/narrowing-logical-operand-edges/tsconfig.json'),
  'narrowing-in-keyword-presence': path.join(workspaceRoot, 'tests/compat-projects/narrowing-in-keyword-presence/tsconfig.json'),
  'narrowing-aliased-conditions': path.join(workspaceRoot, 'tests/compat-projects/narrowing-aliased-conditions/tsconfig.json'),
  'narrowing-predicate-property-argument': path.join(workspaceRoot, 'tests/compat-projects/narrowing-predicate-property-argument/tsconfig.json'),
  'narrowing-equality-reference-operands': path.join(workspaceRoot, 'tests/compat-projects/narrowing-equality-reference-operands/tsconfig.json'),
  'if-surviving-branch-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/if-surviving-branch-narrowing-basic/tsconfig.json'),
  'tuple-literal-length-anchor-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-literal-length-anchor-basic/tsconfig.json'),
  'rest-tuple-literal-context-basic': path.join(workspaceRoot, 'tests/compat-projects/rest-tuple-literal-context-basic/tsconfig.json'),
  'class-expression-heritage-basic': path.join(workspaceRoot, 'tests/compat-projects/class-expression-heritage-basic/tsconfig.json'),
  'spread-tuple-literal-basic': path.join(workspaceRoot, 'tests/compat-projects/spread-tuple-literal-basic/tsconfig.json'),
  'overload-implementation-hidden-basic': path.join(workspaceRoot, 'tests/compat-projects/overload-implementation-hidden-basic/tsconfig.json'),
  'annotated-void-return-basic': path.join(workspaceRoot, 'tests/compat-projects/annotated-void-return-basic/tsconfig.json'),
  'optional-discriminant-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-discriminant-narrowing-basic/tsconfig.json'),
  'union-excess-property-basic': path.join(workspaceRoot, 'tests/compat-projects/union-excess-property-basic/tsconfig.json'),
  'union-target-missing-property-basic': path.join(workspaceRoot, 'tests/compat-projects/union-target-missing-property-basic/tsconfig.json'),
  'uninferred-type-parameter-fallback-basic': path.join(workspaceRoot, 'tests/compat-projects/uninferred-type-parameter-fallback-basic/tsconfig.json'),
  'union-alias-callback-return-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/union-alias-callback-return-inference-basic/tsconfig.json'),
  'lookup-error-type-propagation-basic': path.join(workspaceRoot, 'tests/compat-projects/lookup-error-type-propagation-basic/tsconfig.json'),
  'class-method-own-instantiation-return-basic': path.join(workspaceRoot, 'tests/compat-projects/class-method-own-instantiation-return-basic/tsconfig.json'),
  'function-literal-alias-return-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/function-literal-alias-return-inference-basic/tsconfig.json'),
  'string-numeric-index-basic': path.join(workspaceRoot, 'tests/compat-projects/string-numeric-index-basic/tsconfig.json'),
  'ambient-module-precedes-package-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-module-precedes-package-basic/tsconfig.json'),
  'assertion-contextual-object-literal-basic': path.join(workspaceRoot, 'tests/compat-projects/assertion-contextual-object-literal-basic/tsconfig.json'),
  'unannotated-return-jsx-callback-basic': path.join(workspaceRoot, 'tests/compat-projects/unannotated-return-jsx-callback-basic/tsconfig.json'),
  'intersection-disjoint-property-basic': path.join(workspaceRoot, 'tests/compat-projects/intersection-disjoint-property-basic/tsconfig.json'),
  'lazy-inferred-member-basic': path.join(workspaceRoot, 'tests/compat-projects/lazy-inferred-member-basic/tsconfig.json'),
  'interface-method-overload-selection-basic': path.join(workspaceRoot, 'tests/compat-projects/interface-method-overload-selection-basic/tsconfig.json'),
  'recursive-mapped-alias-member-basic': path.join(workspaceRoot, 'tests/compat-projects/recursive-mapped-alias-member-basic/tsconfig.json'),
  'filter-inferred-predicate-basic': path.join(workspaceRoot, 'tests/compat-projects/filter-inferred-predicate-basic/tsconfig.json'),
  'lazy-intersection-reentry-basic': path.join(workspaceRoot, 'tests/compat-projects/lazy-intersection-reentry-basic/tsconfig.json'),
  'block-arrow-return-basic': path.join(workspaceRoot, 'tests/compat-projects/block-arrow-return-basic/tsconfig.json'),
  'generic-static-class-value-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-static-class-value-basic/tsconfig.json'),
  'conditional-parameter-inference-basic': path.join(workspaceRoot, 'tests/compat-projects/conditional-parameter-inference-basic/tsconfig.json'),
  'falsy-narrowing-filter-predicate-basic': path.join(workspaceRoot, 'tests/compat-projects/falsy-narrowing-filter-predicate-basic/tsconfig.json'),
  'ambient-block-import-signature-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-block-import-signature-basic/tsconfig.json'),
  'script-typeof-global-signature-basic': path.join(workspaceRoot, 'tests/compat-projects/script-typeof-global-signature-basic/tsconfig.json'),
  'template-literal-infer-basic': path.join(workspaceRoot, 'tests/compat-projects/template-literal-infer-basic/tsconfig.json'),
  'comma-operand-allow-unreachable-code-basic': path.join(workspaceRoot, 'tests/compat-projects/comma-operand-allow-unreachable-code-basic/tsconfig.json'),
  'signature-implicit-any-basic': path.join(workspaceRoot, 'tests/compat-projects/signature-implicit-any-basic/tsconfig.json'),
  'signature-parameter-typeof-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/signature-parameter-typeof-scope-basic/tsconfig.json'),
  'script-global-values-across-files-basic': path.join(workspaceRoot, 'tests/compat-projects/script-global-values-across-files-basic/tsconfig.json'),
  'var-redeclaration-identity-basic': path.join(workspaceRoot, 'tests/compat-projects/var-redeclaration-identity-basic/tsconfig.json'),
  'tuple-length-literal-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-length-literal-basic/tsconfig.json'),
  'tuple-identity-arity-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-identity-arity-basic/tsconfig.json'),
  'tuple-target-structural-source-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-target-structural-source-basic/tsconfig.json'),
  'tuple-rest-target-positions-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-rest-target-positions-basic/tsconfig.json'),
  'tuple-optional-element-flags-basic': path.join(workspaceRoot, 'tests/compat-projects/tuple-optional-element-flags-basic/tsconfig.json'),
  'tagged-template-generic-call-basic': path.join(workspaceRoot, 'tests/compat-projects/tagged-template-generic-call-basic/tsconfig.json'),
  'static-property-function-initializer-basic': path.join(workspaceRoot, 'tests/compat-projects/static-property-function-initializer-basic/tsconfig.json'),
  'synthetic-default-import-value-basic': path.join(workspaceRoot, 'tests/compat-projects/synthetic-default-import-value-basic/tsconfig.json'),
  'optional-call-lib-member-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-call-lib-member-basic/tsconfig.json'),
  'conditional-readonly-array-infer-basic': path.join(workspaceRoot, 'tests/compat-projects/conditional-readonly-array-infer-basic/tsconfig.json'),
  'async-implicit-return-awaited-basic': path.join(workspaceRoot, 'tests/compat-projects/async-implicit-return-awaited-basic/tsconfig.json'),
  'index-signature-key-slots-basic': path.join(workspaceRoot, 'tests/compat-projects/index-signature-key-slots-basic/tsconfig.json'),
  'inference-nested-union-alias-basic': path.join(workspaceRoot, 'tests/compat-projects/inference-nested-union-alias-basic/tsconfig.json'),
  'generic-body-return-call-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-body-return-call-basic/tsconfig.json'),
  'unannotated-body-return-checked-basic': path.join(workspaceRoot, 'tests/compat-projects/unannotated-body-return-checked-basic/tsconfig.json'),
  'switch-any-discriminant-implicit-return-basic': path.join(workspaceRoot, 'tests/compat-projects/switch-any-discriminant-implicit-return-basic/tsconfig.json'),
  'callable-interface-overload-member-call-basic': path.join(workspaceRoot, 'tests/compat-projects/callable-interface-overload-member-call-basic/tsconfig.json'),
  'inference-reference-structural-basic': path.join(workspaceRoot, 'tests/compat-projects/inference-reference-structural-basic/tsconfig.json'),
  'conditional-infer-interface-pattern-basic': path.join(workspaceRoot, 'tests/compat-projects/conditional-infer-interface-pattern-basic/tsconfig.json'),
  'inference-fixed-tuple-rest-basic': path.join(workspaceRoot, 'tests/compat-projects/inference-fixed-tuple-rest-basic/tsconfig.json'),
  'recursive-alias-deferred-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/recursive-alias-deferred-reference-basic/tsconfig.json'),
  'umd-global-namespace-type-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/umd-global-namespace-type-reference-basic/tsconfig.json'),
  'export-equals-namespace-top-level-scope-basic': path.join(workspaceRoot, 'tests/compat-projects/export-equals-namespace-top-level-scope-basic/tsconfig.json'),
  'lazy-export-nested-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/lazy-export-nested-reference-basic/tsconfig.json'),
  'optional-chain-truthy-contained-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/optional-chain-truthy-contained-reference-basic/tsconfig.json'),
  'generic-function-value-binding-call-basic': path.join(workspaceRoot, 'tests/compat-projects/generic-function-value-binding-call-basic/tsconfig.json'),
  'modifier-ambient-and-async-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/modifier-ambient-and-async-placement-basic/tsconfig.json'),
  'modifier-combinations-basic': path.join(workspaceRoot, 'tests/compat-projects/modifier-combinations-basic/tsconfig.json'),
  'const-type-parameter-owner-basic': path.join(workspaceRoot, 'tests/compat-projects/const-type-parameter-owner-basic/tsconfig.json'),
  'module-element-context-basic': path.join(workspaceRoot, 'tests/compat-projects/module-element-context-basic/tsconfig.json'),
  'export-assignment-context-basic': path.join(workspaceRoot, 'tests/compat-projects/export-assignment-context-basic/tsconfig.json'),
  'namespace-default-export-basic': path.join(workspaceRoot, 'tests/compat-projects/namespace-default-export-basic/tsconfig.json'),
  'ambient-module-nesting-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-module-nesting-basic/tsconfig.json'),
  'module-keyword-namespace-basic': path.join(workspaceRoot, 'tests/compat-projects/module-keyword-namespace-basic/tsconfig.json'),
  'global-augmentation-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/global-augmentation-placement-basic/tsconfig.json'),
  'ambient-module-export-modifier-basic': path.join(workspaceRoot, 'tests/compat-projects/ambient-module-export-modifier-basic/tsconfig.json'),
  'declaration-file-declare-modifier-basic': path.join(workspaceRoot, 'tests/compat-projects/declaration-file-declare-modifier-basic/tsconfig.json'),
  'use-strict-parameter-list-basic': path.join(workspaceRoot, 'tests/compat-projects/use-strict-parameter-list-basic/tsconfig.json'),
  'super-type-arguments-basic': path.join(workspaceRoot, 'tests/compat-projects/super-type-arguments-basic/tsconfig.json'),
  'instanceof-instantiation-expression-basic': path.join(workspaceRoot, 'tests/compat-projects/instanceof-instantiation-expression-basic/tsconfig.json'),
  'mapped-type-member-in-body-basic': path.join(workspaceRoot, 'tests/compat-projects/mapped-type-member-in-body-basic/tsconfig.json'),
  'arrow-captures-global-this-basic': path.join(workspaceRoot, 'tests/compat-projects/arrow-captures-global-this-basic/tsconfig.json'),
  'computed-type-member-type-name-basic': path.join(workspaceRoot, 'tests/compat-projects/computed-type-member-type-name-basic/tsconfig.json'),
  'for-in-left-operand-basic': path.join(workspaceRoot, 'tests/compat-projects/for-in-left-operand-basic/tsconfig.json'),
  'for-of-es5-lib-basic': path.join(workspaceRoot, 'tests/compat-projects/for-of-es5-lib-basic/tsconfig.json'),
  'missing-dom-lib-property-basic': path.join(workspaceRoot, 'tests/compat-projects/missing-dom-lib-property-basic/tsconfig.json'),
  'object-rest-non-object-basic': path.join(workspaceRoot, 'tests/compat-projects/object-rest-non-object-basic/tsconfig.json'),
  'rest-element-non-array-basic': path.join(workspaceRoot, 'tests/compat-projects/rest-element-non-array-basic/tsconfig.json'),
  'rest-element-property-name-basic': path.join(workspaceRoot, 'tests/compat-projects/rest-element-property-name-basic/tsconfig.json'),
  'setter-implicit-any-basic': path.join(workspaceRoot, 'tests/compat-projects/setter-implicit-any-basic/tsconfig.json'),
  'signature-parameter-named-type-basic': path.join(workspaceRoot, 'tests/compat-projects/signature-parameter-named-type-basic/tsconfig.json'),
  'type-parameter-default-reference-basic': path.join(workspaceRoot, 'tests/compat-projects/type-parameter-default-reference-basic/tsconfig.json'),
  'type-predicate-assignability-basic': path.join(workspaceRoot, 'tests/compat-projects/type-predicate-assignability-basic/tsconfig.json'),
  'unused-label-basic': path.join(workspaceRoot, 'tests/compat-projects/unused-label-basic/tsconfig.json'),
  'enum-reverse-mapping-basic': path.join(workspaceRoot, 'tests/compat-projects/enum-reverse-mapping-basic/tsconfig.json'),
  'reference-lib-directive-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-lib-directive-basic/tsconfig.json'),
  'reference-lib-directive-leading-only-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-lib-directive-leading-only-basic/tsconfig.json'),
  'syntactic-diagnostics-gate-basic': path.join(workspaceRoot, 'tests/compat-projects/syntactic-diagnostics-gate-basic/tsconfig.json'),
  'grammar-diagnostics-do-not-gate-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-diagnostics-do-not-gate-basic/tsconfig.json'),
  'program-diagnostics-gate-basic': path.join(workspaceRoot, 'tests/compat-projects/program-diagnostics-gate-basic/tsconfig.json'),
  'reference-types-diagnostic-does-not-gate-basic': path.join(workspaceRoot, 'tests/compat-projects/reference-types-diagnostic-does-not-gate-basic/tsconfig.json'),
  'property-initialization-literal-names-basic': path.join(workspaceRoot, 'tests/compat-projects/property-initialization-literal-names-basic/tsconfig.json'),
  'parameter-property-placement-basic': path.join(workspaceRoot, 'tests/compat-projects/parameter-property-placement-basic/tsconfig.json'),
  'dotted-namespace-value-member-basic': path.join(workspaceRoot, 'tests/compat-projects/dotted-namespace-value-member-basic/tsconfig.json'),
  'import-equals-entity-alias-basic': path.join(workspaceRoot, 'tests/compat-projects/import-equals-entity-alias-basic/tsconfig.json'),
  'definite-assignment-guard-narrowing-basic': path.join(workspaceRoot, 'tests/compat-projects/definite-assignment-guard-narrowing-basic/tsconfig.json'),
  'script-typeof-class-member-basic': path.join(workspaceRoot, 'tests/compat-projects/script-typeof-class-member-basic/tsconfig.json'),
  'export-clause-local-targets-basic': path.join(workspaceRoot, 'tests/compat-projects/export-clause-local-targets-basic/tsconfig.json'),
  'untyped-javascript-module-basic': path.join(workspaceRoot, 'tests/compat-projects/untyped-javascript-module-basic/tsconfig.json'),
  'untyped-javascript-module-no-implicit-any': path.join(workspaceRoot, 'tests/compat-projects/untyped-javascript-module-no-implicit-any/tsconfig.json'),
  'export-import-require-alias-basic': path.join(workspaceRoot, 'tests/compat-projects/export-import-require-alias-basic/tsconfig.json'),
  'shorthand-ambient-module-basic': path.join(workspaceRoot, 'tests/compat-projects/shorthand-ambient-module-basic/tsconfig.json'),
  'lib-feature-missing-member-basic': path.join(workspaceRoot, 'tests/compat-projects/lib-feature-missing-member-basic/tsconfig.json'),
  'lib-feature-member-present': path.join(workspaceRoot, 'tests/compat-projects/lib-feature-member-present/tsconfig.json'),
  'lib-constructor-resolved-instance': path.join(workspaceRoot, 'tests/compat-projects/lib-constructor-resolved-instance/tsconfig.json'),
  'callable-object-function-members': path.join(workspaceRoot, 'tests/compat-projects/callable-object-function-members/tsconfig.json'),
  'leading-zero-numeric-literal-basic': path.join(workspaceRoot, 'tests/compat-projects/leading-zero-numeric-literal-basic/tsconfig.json'),
  'numeric-literal-forms-valid': path.join(workspaceRoot, 'tests/compat-projects/numeric-literal-forms-valid/tsconfig.json'),
  'var-case-narrowing-scope': path.join(workspaceRoot, 'tests/compat-projects/var-case-narrowing-scope/tsconfig.json'),
  'array-literal-omitted-elements': path.join(workspaceRoot, 'tests/compat-projects/array-literal-omitted-elements/tsconfig.json'),
  'numeric-literal-property-names': path.join(workspaceRoot, 'tests/compat-projects/numeric-literal-property-names/tsconfig.json'),
  'class-static-index-signature': path.join(workspaceRoot, 'tests/compat-projects/class-static-index-signature/tsconfig.json'),
  'numeric-enum-reverse-mapping': path.join(workspaceRoot, 'tests/compat-projects/numeric-enum-reverse-mapping/tsconfig.json'),
  'iife-contextual-parameters': path.join(workspaceRoot, 'tests/compat-projects/iife-contextual-parameters/tsconfig.json'),
  'unused-import-declaration-grouping': path.join(workspaceRoot, 'tests/compat-projects/unused-import-declaration-grouping/tsconfig.json'),
  'default-and-namespace-import': path.join(workspaceRoot, 'tests/compat-projects/default-and-namespace-import/tsconfig.json'),
  'type-only-export-specifier-value': path.join(workspaceRoot, 'tests/compat-projects/type-only-export-specifier-value/tsconfig.json'),
  'jsx-factory-implicit-use': path.join(workspaceRoot, 'tests/compat-projects/jsx-factory-implicit-use/tsconfig.json'),
  'nested-destructuring-assignment-targets': path.join(workspaceRoot, 'tests/compat-projects/nested-destructuring-assignment-targets/tsconfig.json'),
  'binding-element-default-implicit-any': path.join(workspaceRoot, 'tests/compat-projects/binding-element-default-implicit-any/tsconfig.json'),
  'union-tuple-rest-contextual-parameters': path.join(workspaceRoot, 'tests/compat-projects/union-tuple-rest-contextual-parameters/tsconfig.json'),
  'dependent-destructuring-own-guards': path.join(workspaceRoot, 'tests/compat-projects/dependent-destructuring-own-guards/tsconfig.json'),
  'super-type-arguments-parse-error': path.join(workspaceRoot, 'tests/compat-projects/super-type-arguments-parse-error/tsconfig.json'),
  'constructor-guard-definite-assignment': path.join(workspaceRoot, 'tests/compat-projects/constructor-guard-definite-assignment/tsconfig.json'),
  'new-function-uninferred-return': path.join(workspaceRoot, 'tests/compat-projects/new-function-uninferred-return/tsconfig.json'),
  'signature-collection-restores-scope': path.join(workspaceRoot, 'tests/compat-projects/signature-collection-restores-scope/tsconfig.json'),
  'unused-declaration-list-grouping': path.join(workspaceRoot, 'tests/compat-projects/unused-declaration-list-grouping/tsconfig.json'),
  'destructuring-missing-property-at-element': path.join(workspaceRoot, 'tests/compat-projects/destructuring-missing-property-at-element/tsconfig.json'),
  'nullish-right-operand-context-only': path.join(workspaceRoot, 'tests/compat-projects/nullish-right-operand-context-only/tsconfig.json'),
  'function-namespace-merge-members': path.join(workspaceRoot, 'tests/compat-projects/function-namespace-merge-members/tsconfig.json'),
  'local-enum-used-before-declaration': path.join(workspaceRoot, 'tests/compat-projects/local-enum-used-before-declaration/tsconfig.json'),
  'element-access-missing-key-selection': path.join(workspaceRoot, 'tests/compat-projects/element-access-missing-key-selection/tsconfig.json'),
  'import-require-in-namespace-grammar': path.join(workspaceRoot, 'tests/compat-projects/import-require-in-namespace-grammar/tsconfig.json'),
  'annotated-destructuring-declaration': path.join(workspaceRoot, 'tests/compat-projects/annotated-destructuring-declaration/tsconfig.json'),
  'catch-annotation-resolved-type': path.join(workspaceRoot, 'tests/compat-projects/catch-annotation-resolved-type/tsconfig.json'),
  'unresolved-import-type-arguments': path.join(workspaceRoot, 'tests/compat-projects/unresolved-import-type-arguments/tsconfig.json'),
  'bare-super-parse-error': path.join(workspaceRoot, 'tests/compat-projects/bare-super-parse-error/tsconfig.json'),
  'union-rest-signature-call': path.join(workspaceRoot, 'tests/compat-projects/union-rest-signature-call/tsconfig.json'),
  'tuple-union-indexed-access-type': path.join(workspaceRoot, 'tests/compat-projects/tuple-union-indexed-access-type/tsconfig.json'),
  'for-await-in-parse-error': path.join(workspaceRoot, 'tests/compat-projects/for-await-in-parse-error/tsconfig.json'),
  'missing-return-loose-null-checks': path.join(workspaceRoot, 'tests/compat-projects/missing-return-loose-null-checks/tsconfig.json'),
  'script-parameter-default-values': path.join(workspaceRoot, 'tests/compat-projects/script-parameter-default-values/tsconfig.json'),
  'array-literal-spread-arguments': path.join(workspaceRoot, 'tests/compat-projects/array-literal-spread-arguments/tsconfig.json'),
  'plus-nullish-operands-loose': path.join(workspaceRoot, 'tests/compat-projects/plus-nullish-operands-loose/tsconfig.json'),
  'delete-operand-loose-null-checks': path.join(workspaceRoot, 'tests/compat-projects/delete-operand-loose-null-checks/tsconfig.json'),
  'node-modules-root-file': path.join(workspaceRoot, 'tests/compat-projects/node-modules-root-file/tsconfig.json'),
  'node-modules-root-file-esm': path.join(workspaceRoot, 'tests/compat-projects/node-modules-root-file-esm/tsconfig.json'),
  'explicit-ts-extension-imports': path.join(workspaceRoot, 'tests/compat-projects/explicit-ts-extension-imports/tsconfig.json'),
  'explicit-ts-extension-imports-disallowed': path.join(workspaceRoot, 'tests/compat-projects/explicit-ts-extension-imports-disallowed/tsconfig.json'),
  'explicit-ts-extension-imports-rewrite': path.join(workspaceRoot, 'tests/compat-projects/explicit-ts-extension-imports-rewrite/tsconfig.json'),
  'arbitrary-extension-declaration-file': path.join(workspaceRoot, 'tests/compat-projects/arbitrary-extension-declaration-file/tsconfig.json'),
  'arbitrary-extension-imports': path.join(workspaceRoot, 'tests/compat-projects/arbitrary-extension-imports/tsconfig.json'),
  'arbitrary-extension-imports-allowed': path.join(workspaceRoot, 'tests/compat-projects/arbitrary-extension-imports-allowed/tsconfig.json'),
  'ts-nocheck-typescript-file': path.join(workspaceRoot, 'tests/compat-projects/ts-nocheck-typescript-file/tsconfig.json'),
  'contextual-and-comma-operands': path.join(workspaceRoot, 'tests/compat-projects/contextual-and-comma-operands/tsconfig.json'),
  'logical-and-result-type-loose': path.join(workspaceRoot, 'tests/compat-projects/logical-and-result-type-loose/tsconfig.json'),
  'array-binding-parameter-defaults': path.join(workspaceRoot, 'tests/compat-projects/array-binding-parameter-defaults/tsconfig.json'),
  'missing-return-bare-return': path.join(workspaceRoot, 'tests/compat-projects/missing-return-bare-return/tsconfig.json'),
  'ambient-initializer-constants': path.join(workspaceRoot, 'tests/compat-projects/ambient-initializer-constants/tsconfig.json'),
  'jsx-this-tag-value': path.join(workspaceRoot, 'tests/compat-projects/jsx-this-tag-value/tsconfig.json'),
  'namespace-require-alias': path.join(workspaceRoot, 'tests/compat-projects/namespace-require-alias/tsconfig.json'),
  'readonly-array-assignment-code': path.join(workspaceRoot, 'tests/compat-projects/readonly-array-assignment-code/tsconfig.json'),
  'union-accessor-element-write': path.join(workspaceRoot, 'tests/compat-projects/union-accessor-element-write/tsconfig.json'),
  'never-call-statement-exits': path.join(workspaceRoot, 'tests/compat-projects/never-call-statement-exits/tsconfig.json'),
  'destructuring-assignment-computed-key': path.join(workspaceRoot, 'tests/compat-projects/destructuring-assignment-computed-key/tsconfig.json'),
  'destructuring-declaration-computed-key': path.join(workspaceRoot, 'tests/compat-projects/destructuring-declaration-computed-key/tsconfig.json'),
  'generator-return-type-argument': path.join(workspaceRoot, 'tests/compat-projects/generator-return-type-argument/tsconfig.json'),
  'module-element-wrong-context': path.join(workspaceRoot, 'tests/compat-projects/module-element-wrong-context/tsconfig.json'),
  'with-statement-body-unchecked': path.join(workspaceRoot, 'tests/compat-projects/with-statement-body-unchecked/tsconfig.json'),
  'decorator-targets-legacy': path.join(workspaceRoot, 'tests/compat-projects/decorator-targets-legacy/tsconfig.json'),
  'decorator-targets-es': path.join(workspaceRoot, 'tests/compat-projects/decorator-targets-es/tsconfig.json'),
  'jsdoc-nullability-in-typescript': path.join(workspaceRoot, 'tests/compat-projects/jsdoc-nullability-in-typescript/tsconfig.json'),
  'jsdoc-unknown-type-in-typescript': path.join(workspaceRoot, 'tests/compat-projects/jsdoc-unknown-type-in-typescript/tsconfig.json'),
  'untyped-function-subtype-calls': path.join(workspaceRoot, 'tests/compat-projects/untyped-function-subtype-calls/tsconfig.json'),
  'comma-operator-side-effects': path.join(workspaceRoot, 'tests/compat-projects/comma-operator-side-effects/tsconfig.json'),
  'javascript-script-globals': path.join(workspaceRoot, 'tests/compat-projects/javascript-script-globals/tsconfig.json'),
  'javascript-object-literals-open-ended': path.join(workspaceRoot, 'tests/compat-projects/javascript-object-literals-open-ended/tsconfig.json'),
  'javascript-object-literals-no-implicit-any': path.join(workspaceRoot, 'tests/compat-projects/javascript-object-literals-no-implicit-any/tsconfig.json'),
  'javascript-this-assigned-members': path.join(workspaceRoot, 'tests/compat-projects/javascript-this-assigned-members/tsconfig.json'),
  'global-this-script-globals': path.join(workspaceRoot, 'tests/compat-projects/global-this-script-globals/tsconfig.json'),
  'optional-chain-element-call': path.join(workspaceRoot, 'tests/compat-projects/optional-chain-element-call/tsconfig.json'),
  'destructuring-assignment-in-comma': path.join(workspaceRoot, 'tests/compat-projects/destructuring-assignment-in-comma/tsconfig.json'),
  'jsdoc-link-counts-as-use': path.join(workspaceRoot, 'tests/compat-projects/jsdoc-link-counts-as-use/tsconfig.json'),
  'accessor-read-write-accessibility': path.join(workspaceRoot, 'tests/compat-projects/accessor-read-write-accessibility/tsconfig.json'),
  'labeled-const-declaration-context': path.join(workspaceRoot, 'tests/compat-projects/labeled-const-declaration-context/tsconfig.json'),
  'string-slice-arity': path.join(workspaceRoot, 'tests/compat-projects/string-slice-arity/tsconfig.json'),
  'protected-access-this-parameter': path.join(workspaceRoot, 'tests/compat-projects/protected-access-this-parameter/tsconfig.json'),
  'protected-access-through-instance': path.join(workspaceRoot, 'tests/compat-projects/protected-access-through-instance/tsconfig.json'),
  'no-check-skips-type-checking': path.join(workspaceRoot, 'tests/compat-projects/no-check-skips-type-checking/tsconfig.json'),
  'call-first-failing-argument': path.join(workspaceRoot, 'tests/compat-projects/call-first-failing-argument/tsconfig.json'),
  'parameter-nullish-initializer': path.join(workspaceRoot, 'tests/compat-projects/parameter-nullish-initializer/tsconfig.json'),
  'computed-name-index-contextual-type': path.join(workspaceRoot, 'tests/compat-projects/computed-name-index-contextual-type/tsconfig.json'),
  'assignment-contextual-callback': path.join(workspaceRoot, 'tests/compat-projects/assignment-contextual-callback/tsconfig.json'),
  'loose-null-widening': path.join(workspaceRoot, 'tests/compat-projects/loose-null-widening/tsconfig.json'),
  'optional-call-chain-nullability': path.join(workspaceRoot, 'tests/compat-projects/optional-call-chain-nullability/tsconfig.json'),
  'module-member-assignment-narrowing': path.join(workspaceRoot, 'tests/compat-projects/module-member-assignment-narrowing/tsconfig.json'),
  'satisfies-argument-elaboration': path.join(workspaceRoot, 'tests/compat-projects/satisfies-argument-elaboration/tsconfig.json'),
  'new-error-type-callee': path.join(workspaceRoot, 'tests/compat-projects/new-error-type-callee/tsconfig.json'),
  'function-without-return-is-void': path.join(workspaceRoot, 'tests/compat-projects/function-without-return-is-void/tsconfig.json'),
  'override-method-bivariance': path.join(workspaceRoot, 'tests/compat-projects/override-method-bivariance/tsconfig.json'),
  'method-this-parameter': path.join(workspaceRoot, 'tests/compat-projects/method-this-parameter/tsconfig.json'),
  'ambient-destructuring-declaration': path.join(workspaceRoot, 'tests/compat-projects/ambient-destructuring-declaration/tsconfig.json'),
  'iife-spread-type-variable': path.join(workspaceRoot, 'tests/compat-projects/iife-spread-type-variable/tsconfig.json'),
  'reference-path-unresolved': path.join(workspaceRoot, 'tests/compat-projects/reference-path-unresolved/tsconfig.json'),
  'reference-path-skip-lib-check': path.join(workspaceRoot, 'tests/compat-projects/reference-path-skip-lib-check/tsconfig.json'),
  'reference-path-no-resolve': path.join(workspaceRoot, 'tests/compat-projects/reference-path-no-resolve/tsconfig.json'),
  'super-type-arguments-follower': path.join(workspaceRoot, 'tests/compat-projects/super-type-arguments-follower/tsconfig.json'),
  'update-expression-write-target': path.join(workspaceRoot, 'tests/compat-projects/update-expression-write-target/tsconfig.json'),
  'update-operand-arithmetic-first': path.join(workspaceRoot, 'tests/compat-projects/update-operand-arithmetic-first/tsconfig.json'),
  'use-strict-parameter-list-es2015': path.join(workspaceRoot, 'tests/compat-projects/use-strict-parameter-list-es2015/tsconfig.json'),
  'object-literal-predicate-members': path.join(workspaceRoot, 'tests/compat-projects/object-literal-predicate-members/tsconfig.json'),
  'property-reference-discriminant-narrowing': path.join(workspaceRoot, 'tests/compat-projects/property-reference-discriminant-narrowing/tsconfig.json'),
  'property-reference-nullish-union-base': path.join(workspaceRoot, 'tests/compat-projects/property-reference-nullish-union-base/tsconfig.json'),
  'override-declare-property': path.join(workspaceRoot, 'tests/compat-projects/override-declare-property/tsconfig.json'),
  'class-property-arrow-signature': path.join(workspaceRoot, 'tests/compat-projects/class-property-arrow-signature/tsconfig.json'),
  'synthetic-default-overrides-default-export': path.join(workspaceRoot, 'tests/compat-projects/synthetic-default-overrides-default-export/tsconfig.json'),
  'synthetic-default-node-esm-import-cjs': path.join(workspaceRoot, 'tests/compat-projects/synthetic-default-node-esm-import-cjs/tsconfig.json'),
  'synthetic-default-namespace-import-member': path.join(workspaceRoot, 'tests/compat-projects/synthetic-default-namespace-import-member/tsconfig.json'),
  'export-star-as-default': path.join(workspaceRoot, 'tests/compat-projects/export-star-as-default/tsconfig.json'),
  'umd-global-binding': path.join(workspaceRoot, 'tests/compat-projects/umd-global-binding/tsconfig.json'),
  'type-query-declaring-module-scope': path.join(workspaceRoot, 'tests/compat-projects/type-query-declaring-module-scope/tsconfig.json'),
  'include-parent-directory': path.join(workspaceRoot, 'tests/compat-projects/include-parent-directory/app/tsconfig.json'),
  'optional-property-source-index-signature': path.join(workspaceRoot, 'tests/compat-projects/optional-property-source-index-signature/tsconfig.json'),
  'namespace-member-declarations-checked': path.join(workspaceRoot, 'tests/compat-projects/namespace-member-declarations-checked/tsconfig.json'),
  'type-variable-arithmetic-operand': path.join(workspaceRoot, 'tests/compat-projects/type-variable-arithmetic-operand/tsconfig.json'),
  'compound-assignment-not-definite': path.join(workspaceRoot, 'tests/compat-projects/compound-assignment-not-definite/tsconfig.json'),
  'unresolved-type-reference-arguments-checked': path.join(workspaceRoot, 'tests/compat-projects/unresolved-type-reference-arguments-checked/tsconfig.json'),
  'tsc-parser-statement-recovery': path.join(workspaceRoot, 'tests/compat-projects/tsc-parser-statement-recovery/tsconfig.json'),
  'tsc-scanner-errors': path.join(workspaceRoot, 'tests/compat-projects/tsc-scanner-errors/tsconfig.json'),
  'tsc-parser-jsx-recovery': path.join(workspaceRoot, 'tests/compat-projects/tsc-parser-jsx-recovery/tsconfig.json'),
  'tsc-parser-javascript-only-syntax': path.join(workspaceRoot, 'tests/compat-projects/tsc-parser-javascript-only-syntax/tsconfig.json'),
  'script-namespace-qualified-members': path.join(workspaceRoot, 'tests/compat-projects/script-namespace-qualified-members/tsconfig.json'),
  'tsc-binder-declarations': path.join(workspaceRoot, 'tests/compat-projects/tsc-binder-declarations/tsconfig.json'),
  'error-call-operands-checked': path.join(workspaceRoot, 'tests/compat-projects/error-call-operands-checked/tsconfig.json'),
  'tsc-regular-expressions': path.join(workspaceRoot, 'tests/compat-projects/tsc-regular-expressions/tsconfig.json'),
  'decorator-expressions-checked': path.join(workspaceRoot, 'tests/compat-projects/decorator-expressions-checked/tsconfig.json'),
  'write-targets-checked': path.join(workspaceRoot, 'tests/compat-projects/write-targets-checked/tsconfig.json'),
  'var-redeclaration-types': path.join(workspaceRoot, 'tests/compat-projects/var-redeclaration-types/tsconfig.json'),
  'element-reference-narrowing': path.join(workspaceRoot, 'tests/compat-projects/element-reference-narrowing/tsconfig.json'),
  'method-return-inferred': path.join(workspaceRoot, 'tests/compat-projects/method-return-inferred/tsconfig.json'),
  'enum-literal-widening': path.join(workspaceRoot, 'tests/compat-projects/enum-literal-widening/tsconfig.json'),
  'checker-grammar-rejections': path.join(workspaceRoot, 'tests/compat-projects/checker-grammar-rejections/tsconfig.json'),
  'erasable-syntax-only': path.join(workspaceRoot, 'tests/compat-projects/erasable-syntax-only/tsconfig.json'),
  'unreachable-never-calls': path.join(workspaceRoot, 'tests/compat-projects/unreachable-never-calls/tsconfig.json'),
  'module-es2015-import-forms': path.join(workspaceRoot, 'tests/compat-projects/module-es2015-import-forms/tsconfig.json'),
  'class-member-duplicates': path.join(workspaceRoot, 'tests/compat-projects/class-member-duplicates/tsconfig.json'),
  'signature-pattern-implicit-any': path.join(workspaceRoot, 'tests/compat-projects/signature-pattern-implicit-any/tsconfig.json'),
  'namespace-import-member-writes': path.join(workspaceRoot, 'tests/compat-projects/namespace-import-member-writes/tsconfig.json'),
  'member-write-values-checked': path.join(workspaceRoot, 'tests/compat-projects/member-write-values-checked/tsconfig.json'),
  'assignment-reduction-rules': path.join(workspaceRoot, 'tests/compat-projects/assignment-reduction-rules/tsconfig.json'),
  'keyof-non-public-members': path.join(workspaceRoot, 'tests/compat-projects/keyof-non-public-members/tsconfig.json'),
  'this-before-super-flow': path.join(workspaceRoot, 'tests/compat-projects/this-before-super-flow/tsconfig.json'),
  'decorator-iife-use-before-declaration': path.join(workspaceRoot, 'tests/compat-projects/decorator-iife-use-before-declaration/tsconfig.json'),
  'super-abstract-member-access': path.join(workspaceRoot, 'tests/compat-projects/super-abstract-member-access/tsconfig.json'),
  'unused-identifiers-by-symbol': path.join(workspaceRoot, 'tests/compat-projects/unused-identifiers-by-symbol/tsconfig.json'),
  'heritage-cycles-and-conflicts': path.join(workspaceRoot, 'tests/compat-projects/heritage-cycles-and-conflicts/tsconfig.json'),
  'variable-declaration-list-grammar': path.join(workspaceRoot, 'tests/compat-projects/variable-declaration-list-grammar/tsconfig.json'),
  'binding-patterns-and-nested-classes': path.join(workspaceRoot, 'tests/compat-projects/binding-patterns-and-nested-classes/tsconfig.json'),
  'namespace-values-and-generators': path.join(workspaceRoot, 'tests/compat-projects/namespace-values-and-generators/tsconfig.json'),
  'javascript-jsdoc-and-commonjs': path.join(workspaceRoot, 'tests/compat-projects/javascript-jsdoc-and-commonjs/tsconfig.json'),
  'mapped-type-member-grammar': path.join(workspaceRoot, 'tests/compat-projects/mapped-type-member-grammar/tsconfig.json'),
  'override-and-modifier-order': path.join(workspaceRoot, 'tests/compat-projects/override-and-modifier-order/tsconfig.json'),
  'isolated-declarations': path.join(workspaceRoot, 'tests/compat-projects/isolated-declarations/tsconfig.json'),
  'alias-resolution-basic': path.join(workspaceRoot, 'tests/compat-projects/alias-resolution-basic/tsconfig.json'),
  'private-name-diagnostics': path.join(workspaceRoot, 'tests/compat-projects/private-name-diagnostics/tsconfig.json'),
  'weak-type-relations': path.join(workspaceRoot, 'tests/compat-projects/weak-type-relations/tsconfig.json'),
  'node16-module-format-checks': path.join(workspaceRoot, 'tests/compat-projects/node16-module-format-checks/tsconfig.json'),
  'javascript-extends-type-arguments': path.join(workspaceRoot, 'tests/compat-projects/javascript-extends-type-arguments/tsconfig.json'),
  'import-type-module-without-type': path.join(workspaceRoot, 'tests/compat-projects/import-type-module-without-type/tsconfig.json'),
  'jsdoc-tag-shape-checks': path.join(workspaceRoot, 'tests/compat-projects/jsdoc-tag-shape-checks/tsconfig.json'),
  'import-call-argument-checks': path.join(workspaceRoot, 'tests/compat-projects/import-call-argument-checks/tsconfig.json'),
  'verbatim-module-syntax-commonjs': path.join(workspaceRoot, 'tests/compat-projects/verbatim-module-syntax-commonjs/tsconfig.json'),
  'import-call-module-options': path.join(workspaceRoot, 'tests/compat-projects/import-call-module-options/tsconfig.json'),
  'jsx-factory-and-type-argument-arity': path.join(workspaceRoot, 'tests/compat-projects/jsx-factory-and-type-argument-arity/tsconfig.json'),
  'import-helpers-missing-tslib': path.join(workspaceRoot, 'tests/compat-projects/import-helpers-missing-tslib/tsconfig.json'),
  'import-helpers-incomplete-tslib': path.join(workspaceRoot, 'tests/compat-projects/import-helpers-incomplete-tslib/tsconfig.json'),
  'enum-computed-member-initializers': path.join(workspaceRoot, 'tests/compat-projects/enum-computed-member-initializers/tsconfig.json'),
  'instantiation-expression-applicability': path.join(workspaceRoot, 'tests/compat-projects/instantiation-expression-applicability/tsconfig.json'),
  'generator-yield-types-nonstrict': path.join(workspaceRoot, 'tests/compat-projects/generator-yield-types-nonstrict/tsconfig.json'),
  'generator-grammar-and-iteration-checks': path.join(workspaceRoot, 'tests/compat-projects/generator-grammar-and-iteration-checks/tsconfig.json'),
  'export-star-and-type-only-namespace': path.join(workspaceRoot, 'tests/compat-projects/export-star-and-type-only-namespace/tsconfig.json'),
  'class-member-grammar-extra': path.join(workspaceRoot, 'tests/compat-projects/class-member-grammar-extra/tsconfig.json'),
  'decorator-return-legacy': path.join(workspaceRoot, 'tests/compat-projects/decorator-return-legacy/tsconfig.json'),
  'grammar-using-declarations-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-using-declarations-basic/tsconfig.json'),
  'grammar-await-context-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-await-context-basic/tsconfig.json'),
  'grammar-top-level-await-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-top-level-await-basic/tsconfig.json'),
  'grammar-rest-trailing-comma-basic': path.join(workspaceRoot, 'tests/compat-projects/grammar-rest-trailing-comma-basic/tsconfig.json'),
  'jsx-factory-option-values-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-factory-option-values-basic/tsconfig.json'),
  'jsx-react-namespace-option-value-basic': path.join(workspaceRoot, 'tests/compat-projects/jsx-react-namespace-option-value-basic/tsconfig.json'),
  'isolated-modules-enum-cross-file-member': path.join(workspaceRoot, 'tests/compat-projects/isolated-modules-enum-cross-file-member/tsconfig.json'),
  'variance-annotation-declared-types-basic': path.join(workspaceRoot, 'tests/compat-projects/variance-annotation-declared-types-basic/tsconfig.json'),
  'constructor-return-basic': path.join(workspaceRoot, 'tests/compat-projects/constructor-return-basic/tsconfig.json'),
  'get-accessor-call-basic': path.join(workspaceRoot, 'tests/compat-projects/get-accessor-call-basic/tsconfig.json'),
  'readonly-index-signature-writes': path.join(workspaceRoot, 'tests/compat-projects/readonly-index-signature-writes/tsconfig.json'),
  'import-attribute-value-literals': path.join(workspaceRoot, 'tests/compat-projects/import-attribute-value-literals/tsconfig.json'),
  'logical-truthiness-and-literal-checks': path.join(workspaceRoot, 'tests/compat-projects/logical-truthiness-and-literal-checks/tsconfig.json'),
  'jsx-tag-signature-checks': path.join(workspaceRoot, 'tests/compat-projects/jsx-tag-signature-checks/tsconfig.json'),
  'jsx-spread-child-factory-option': path.join(workspaceRoot, 'tests/compat-projects/jsx-spread-child-factory-option/tsconfig.json'),
  'jsx-fragment-factory-pragma': path.join(workspaceRoot, 'tests/compat-projects/jsx-fragment-factory-pragma/tsconfig.json'),
  'jsx-spread-attribute-checks': path.join(workspaceRoot, 'tests/compat-projects/jsx-spread-attribute-checks/tsconfig.json'),
  'unresolved-import-type-references': path.join(workspaceRoot, 'tests/compat-projects/unresolved-import-type-references/tsconfig.json'),
  'jsx-element-grammar-checks': path.join(workspaceRoot, 'tests/compat-projects/jsx-element-grammar-checks/tsconfig.json'),
};

export function main(argv = process.argv.slice(2)): void {
  const args = parseArgs(argv);
  const mode = resolveOracleMode(args);
  const comparison =
    mode.kind === 'project'
      ? compareProject(
          mode.resolvedTsconfig,
          displayComparisonTargetPath(mode.resolvedTsconfig),
          args.maxDiagnostics,
          mode.stubExternalModules,
          mode.rustJobs,
        )
      : compareFile(mode.resolvedFile, displayComparisonTargetPath(mode.resolvedFile), args.maxDiagnostics, mode.ignoreConfig, mode.stubExternalModules);

  if (args.json) {
    process.stdout.write(`${JSON.stringify(comparison, null, 2)}\n`);
  } else {
    process.stdout.write(renderComparisonText(comparison));
  }

  const hasMismatch = !comparison.summary.byCodeMatch || !comparison.summary.byFileCodeMatch;
  const hasMessageMismatch = comparison.summary.messageMatch === false;
  if ((args.failOnMismatch && hasMismatch) || (args.strictMessages && hasMessageMismatch)) {
    process.exitCode = 1;
  }
}

export function parseArgs(argv: string[]): ParsedArgs {
  const parsed: ParsedArgs = {
    json: false,
    failOnMismatch: false,
    strictMessages: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];

    if (arg === '--help' || arg === '-h') {
      printHelpAndExit();
    } else if (arg === '--') {
      continue;
    } else if (arg === '--project' || arg === '--fixture') {
      const value = argv[++index];
      if (!value) {
        throw new Error(`${arg} requires a value`);
      }
      parsed.projectInput = value;
    } else if (arg === '--file') {
      const value = argv[++index];
      if (!value) {
        throw new Error('--file requires a value');
      }
      parsed.fileInput = value;
    } else if (arg === '--json') {
      parsed.json = true;
    } else if (arg === '--ignoreConfig') {
      parsed.ignoreConfig = true;
    } else if (arg === '--stubExternalModules') {
      parsed.stubExternalModules = true;
    } else if (arg === '--rustJobs') {
      const value = argv[++index];
      if (!value) {
        throw new Error('--rustJobs requires a value');
      }
      const parsedValue = Number(value);
      if (!Number.isInteger(parsedValue) || parsedValue <= 0) {
        throw new Error('--rustJobs must be greater than 0');
      }
      parsed.rustJobs = parsedValue;
    } else if (arg === '--failOnMismatch' || arg === '--strictCodes') {
      parsed.failOnMismatch = true;
    } else if (arg === '--strictMessages') {
      parsed.strictMessages = true;
    } else if (arg === '--maxDiagnostics') {
      const value = argv[++index];
      if (!value) {
        throw new Error('--maxDiagnostics requires a value');
      }
      const parsedValue = Number(value);
      if (!Number.isInteger(parsedValue) || parsedValue <= 0) {
        throw new Error('--maxDiagnostics must be greater than 0');
      }
      parsed.maxDiagnostics = parsedValue;
    } else if (arg.startsWith('--')) {
      throw new Error(`unknown argument: ${arg}`);
    } else {
      throw new Error(`unexpected positional argument: ${arg}. Use --project <path-or-preset> or --file <path>.`);
    }
  }

  return parsed;
}

export function resolveOracleMode(args: ParsedArgs): OracleMode {
  const hasProject = args.projectInput !== undefined;
  const hasFile = args.fileInput !== undefined;

  if (hasProject === hasFile) {
    throw new Error('choose exactly one of --project or --file.');
  }

  if (hasProject) {
    if (args.ignoreConfig) {
      throw new Error('--ignoreConfig is only supported with --file in the oracle.');
    }
    const projectInput = args.projectInput as string;
    const resolvedTsconfig = resolveProjectPresetOrPath(projectInput);
    return {
      kind: 'project',
      project: projectInput,
      resolvedTsconfig,
      stubExternalModules: args.stubExternalModules,
      rustJobs: args.rustJobs,
    };
  }

  if (args.rustJobs !== undefined) {
    throw new Error('--rustJobs is only supported with --project.');
  }

  const fileInput = args.fileInput as string;
  const resolvedFile = resolveFilePath(fileInput);
  return {
    kind: 'file',
    file: fileInput,
    resolvedFile,
    ignoreConfig: args.ignoreConfig,
    stubExternalModules: args.stubExternalModules,
  };
}

export function resolveProjectPresetOrPath(projectInput: string): string {
  const preset = fixturePresets[projectInput];
  if (preset) {
    return preset;
  }

  if (isSourceFilePath(projectInput)) {
    throw new Error(
      `--project expects a preset name or tsconfig.json path. For single files, use --file ${projectInput}.`,
    );
  }

  if (isTsConfigPath(projectInput)) {
    const tsconfigPath = resolveWorkspacePath(projectInput);
    if (!existsSync(tsconfigPath) || !statSync(tsconfigPath).isFile()) {
      throw new Error(`missing tsconfig.json at ${normalizePathForDisplay(tsconfigPath)}`);
    }

    return tsconfigPath;
  }

  if (looksLikePath(projectInput)) {
    const resolvedPath = resolveWorkspacePath(projectInput);
    if (existsSync(resolvedPath)) {
      const stats = statSync(resolvedPath);
      if (stats.isDirectory()) {
        const tsconfigPath = path.join(resolvedPath, 'tsconfig.json');
        if (existsSync(tsconfigPath) && statSync(tsconfigPath).isFile()) {
          return tsconfigPath;
        }

        throw new Error(`missing tsconfig.json at ${normalizePathForDisplay(tsconfigPath)}`);
      }

      // A variant config beside the canonical one (`tsconfig.surge.json`, the
      // aggregate a monorepo corpus is measured through) is a tsconfig too.
      if (stats.isFile() && isTsConfigVariantPath(resolvedPath)) {
        return resolvedPath;
      }
    }

    if (projectInput.endsWith('.json')) {
      if (path.basename(projectInput).toLowerCase().includes('tsconfig')) {
        throw new Error(`missing tsconfig.json at ${normalizePathForDisplay(resolvedPath)}`);
      }

      throw new Error(
        `--project expects a preset name or tsconfig.json path. For single files, use --file ${projectInput}.`,
      );
    }

    const tsconfigPath = path.join(resolvedPath, 'tsconfig.json');
    throw new Error(`missing tsconfig.json at ${normalizePathForDisplay(tsconfigPath)}`);
  }

  throw new Error(`unknown oracle project preset: ${projectInput}`);
}

export function resolveFilePath(fileInput: string): string {
  if (isTsConfigPath(fileInput)) {
    throw new Error('--file expects a TypeScript source file, not tsconfig.json. For projects, use --project.');
  }

  if (fileInput.toLowerCase().endsWith('.d.ts')) {
    throw new Error(`--file currently supports .ts source files only. Received ${fileInput}.`);
  }

  const extension = path.extname(fileInput).toLowerCase();
  if (extension !== '.ts') {
    throw new Error(`--file currently supports .ts source files only. Received ${fileInput}.`);
  }

  const resolvedFile = resolveWorkspacePath(fileInput);
  if (!existsSync(resolvedFile) || !statSync(resolvedFile).isFile()) {
    throw new Error(`missing TypeScript source file: ${normalizePathForDisplay(resolvedFile)}`);
  }

  return resolvedFile;
}

export function compareProject(
  tsconfigPath: string,
  projectDisplay: string,
  maxDiagnostics?: number,
  stubExternalModules?: boolean,
  rustJobs?: number,
): ComparisonResult {
  return executeComparison(
    {
      kind: 'project',
      project: projectDisplay,
      resolvedTsconfig: tsconfigPath,
      stubExternalModules,
      rustJobs,
    },
    maxDiagnostics,
  );
}

export function compareFile(
  filePath: string,
  fileDisplay: string,
  maxDiagnostics?: number,
  ignoreConfig?: boolean,
  stubExternalModules?: boolean,
): ComparisonResult {
  return executeComparison(
    {
      kind: 'file',
      file: fileDisplay,
      resolvedFile: filePath,
      ignoreConfig,
      stubExternalModules,
    },
    maxDiagnostics,
  );
}

export function runTsc(mode: OracleMode): RunResult {
  const args =
    mode.kind === 'project'
      ? [oracleTscBinPath, '--noEmit', '--pretty', 'false', '--project', mode.resolvedTsconfig]
      : [oracleTscBinPath, '--noEmit', '--pretty', 'false', mode.resolvedFile];
  if (mode.ignoreConfig) {
      args.splice(args.length - 1, 0, '--ignoreConfig');
  }
  const result = spawnSync(process.execPath, args, {
    cwd: workspaceRoot,
    encoding: 'utf8',
    maxBuffer: subprocessMaxBuffer,
    env: {
      ...process.env,
      npm_config_cache: packageManagerCache,
    },
  });

  if (result.error) {
    throw new Error(`failed to run TypeScript compiler: ${result.error.message}`);
  }

  return {
    exitCode: result.status,
    stdout: result.stdout ?? '',
    stderr: result.stderr ?? '',
  };
}

export function runSurgeTs(
  mode: OracleMode,
  maxDiagnostics?: number,
  rustJobs?: number,
): RunResult {
  const exePath = resolveSurgeBin();
  const args: string[] = [];

  // Argument order mirrors buildSurgeTsCommand so the printed command
  // matches what actually runs. The positional source file must come last.
  if (mode.kind === 'project') {
    args.push('--project', mode.resolvedTsconfig, '--format', 'json');
    if (mode.stubExternalModules) {
      args.push('--stubExternalModules');
    }
    if (rustJobs !== undefined) {
      args.push('--jobs', String(rustJobs));
    }
    if (maxDiagnostics !== undefined) {
      args.push('--maxDiagnostics', String(maxDiagnostics));
    }
  } else {
    args.push('--format', 'json');
    if (mode.ignoreConfig) {
      args.push('--ignoreConfig');
    }
    if (mode.stubExternalModules) {
      args.push('--stubExternalModules');
    }
    if (maxDiagnostics !== undefined) {
      args.push('--maxDiagnostics', String(maxDiagnostics));
    }
    args.push(mode.resolvedFile);
  }

  const result = spawnSync(exePath, args, {
    cwd: workspaceRoot,
    encoding: 'utf8',
    maxBuffer: subprocessMaxBuffer,
  });

  if (result.error) {
    throw new Error(`failed to run surge-ts-cli: ${result.error.message}`);
  }

  return {
    exitCode: result.status,
    stdout: result.stdout ?? '',
    stderr: result.stderr ?? '',
  };
}

export function parseTypeScriptDiagnostics(output: string, projectDir: string): NormalizedDiagnostic[] {
  const diagnostics: NormalizedDiagnostic[] = [];
  const lines = output.split(/\r?\n/);

  for (const rawLine of lines) {
    const line = rawLine.trimEnd();
    if (!line) {
      continue;
    }

    // A message chain continues on lines indented beneath its head.
    const last = diagnostics.at(-1);
    if (last && /^\s+\S/.test(line)) {
      last.message = `${last.message ?? ''}\n${line}`;
      continue;
    }

    const fileDiagnostic = line.match(/^(.*)\((\d+),(\d+)\): error (TS\d+): (.*)$/);
    if (fileDiagnostic) {
      diagnostics.push({
        source: 'typescript',
        fileName: normalizeDiagnosticFileName(projectDir, fileDiagnostic[1]),
        line: Number(fileDiagnostic[2]),
        column: Number(fileDiagnostic[3]),
        code: fileDiagnostic[4],
        message: fileDiagnostic[5],
      });
      continue;
    }

    const globalDiagnostic = line.match(/^error (TS\d+): (.*)$/);
    if (globalDiagnostic) {
      diagnostics.push({
        source: 'typescript',
        fileName: '',
        code: globalDiagnostic[1],
        message: globalDiagnostic[2],
      });
    }
  }

  return diagnostics;
}

export function parseSurgeTsDiagnostics(
  output: string,
  projectDir: string,
): NormalizedDiagnostic[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(output);
  } catch (error) {
    throw new Error(
      `surge-ts-cli did not emit valid JSON diagnostics.\n${formatParseFailure(output, error)}`,
    );
  }

  const diagnostics = Array.isArray((parsed as { diagnostics?: unknown }).diagnostics)
    ? ((parsed as { diagnostics: unknown[] }).diagnostics ?? [])
    : [];

  return diagnostics.map((diagnostic) => {
    const entry = diagnostic as {
      code?: unknown;
      fileName?: unknown;
      line?: unknown;
      column?: unknown;
      message?: unknown;
    };

    return {
      source: 'surge-ts',
      code: String(entry.code ?? ''),
      fileName: normalizeDiagnosticFileName(projectDir, String(entry.fileName ?? '')),
      line: typeof entry.line === 'number' ? entry.line : undefined,
      column: typeof entry.column === 'number' ? entry.column : undefined,
      message: typeof entry.message === 'string' ? entry.message : undefined,
    };
  });
}

export function normalizeDiagnosticFileName(projectDir: string, fileName: string): string {
  if (!fileName) {
    return '';
  }

  const normalizedWorkspaceRoot = normalizePathForDisplay(workspaceRoot).replace(/\/+$/, '');
  const normalizedProjectDir = isAbsolutePathLike(projectDir)
    ? normalizePathForDisplay(projectDir).replace(/\/+$/, '')
    : normalizePathForDisplay(path.resolve(projectDir)).replace(/\/+$/, '');
  const workspaceRelativeProjectDir = normalizePathForDisplay(
    path.relative(workspaceRoot, isAbsolutePathLike(projectDir) ? projectDir : path.resolve(projectDir)),
  ).replace(/\/+$/, '');
  const normalizedInputFileName = normalizePathForDisplay(fileName);

  if (
    workspaceRelativeProjectDir &&
    normalizedInputFileName.startsWith(`${workspaceRelativeProjectDir}/`)
  ) {
    return normalizedInputFileName.slice(workspaceRelativeProjectDir.length + 1);
  }

  if (workspaceRelativeProjectDir && normalizedInputFileName === workspaceRelativeProjectDir) {
    return path.basename(normalizedInputFileName);
  }

  let normalizedFileName = isAbsolutePathLike(fileName)
    ? normalizedInputFileName
    : normalizePathForDisplay(`${normalizedProjectDir}/${normalizedInputFileName}`);

  if (normalizedFileName.startsWith(`${normalizedWorkspaceRoot}/`)) {
    normalizedFileName = normalizedFileName.slice(normalizedWorkspaceRoot.length + 1);
  }

  if (workspaceRelativeProjectDir && normalizedFileName === workspaceRelativeProjectDir) {
    return path.basename(normalizedFileName);
  }

  if (workspaceRelativeProjectDir && normalizedFileName.startsWith(`${workspaceRelativeProjectDir}/`)) {
    return normalizedFileName.slice(workspaceRelativeProjectDir.length + 1);
  }

  if (normalizedFileName === normalizedProjectDir) {
    return path.basename(normalizedFileName);
  }

  const projectPrefix = `${normalizedProjectDir}/`;
  if (normalizedProjectDir && normalizedFileName.startsWith(projectPrefix)) {
    return normalizedFileName.slice(projectPrefix.length);
  }

  return normalizedFileName;
}

export function normalizePathForDisplay(value: string): string {
  return value.replace(/\\/g, '/');
}

export function limitDiagnostics(
  diagnostics: NormalizedDiagnostic[],
  maxDiagnostics?: number,
): NormalizedDiagnostic[] {
  if (maxDiagnostics === undefined) {
    return diagnostics;
  }

  return diagnostics.slice(0, maxDiagnostics);
}

export function normalizeDiagnostic(diagnostic: NormalizedDiagnostic): DiagnosticFingerprint {
  return {
    fileName: diagnostic.fileName,
    code: diagnostic.code,
    line: diagnostic.line ?? null,
    column: diagnostic.column ?? null,
    message: diagnostic.message ?? null,
  };
}

export function compareDiagnostics(
  mode: 'project' | 'file',
  targetDisplay: string,
  typescript: NormalizedDiagnostic[],
  surgeTs: NormalizedDiagnostic[],
  ignoreConfig?: boolean,
  stubExternalModules?: boolean,
  rustJobs?: number,
): ComparisonResult {
  const byCode = compareBuckets(typescript, surgeTs, keyByCode);
  const byFileCode = compareBuckets(typescript, surgeTs, keyByFileCode);
  const byFileCodeLine = compareBuckets(
    typescript.filter(hasLineInfo),
    surgeTs.filter(hasLineInfo),
    keyByFileCodeLine,
  );
  const onlyDiagnostics = subtractDiagnosticsByKey(
    typescript,
    surgeTs,
    keyByDiagnosticFingerprint,
  );
  const onlyTypeScriptDiagnostics = onlyDiagnostics.onlyLeft;
  const onlySurgeTsDiagnostics = onlyDiagnostics.onlyRight;
  const messageParity = compareMessages(typescript, surgeTs);

  return {
    mode,
    project: mode === 'project' ? targetDisplay : null,
    file: mode === 'file' ? targetDisplay : null,
    ignoreConfig: ignoreConfig ?? false,
    surgeTsOptions: {
      stubExternalModules: stubExternalModules ?? false,
      rustJobs,
    },
    warnings: buildComparisonWarnings(typescript, surgeTs),
    tooling: {
      typescriptVersion: pinnedTypeScriptVersion,
      typescriptCommand: buildTypeScriptCommand(mode, targetDisplay, ignoreConfig),
      surgeTsCommand: buildSurgeTsCommand(
        mode,
        targetDisplay,
        ignoreConfig,
        stubExternalModules,
        rustJobs,
      ),
      surgeTsJobs: rustJobs,
    },
    typescript: summarizeDiagnostics(typescript),
    surgeTs: summarizeDiagnostics(surgeTs),
    matches: {
      byCode: byCode.matches,
      onlyTypeScript: byCode.onlyTypeScript,
      onlySurgeTs: byCode.onlySurgeTs,
      byFileCode: byFileCode.matches,
      onlyTypeScriptFileCode: byFileCode.onlyTypeScript,
      onlySurgeTsFileCode: byFileCode.onlySurgeTs,
      byFileCodeLine: byFileCodeLine.matches,
      onlyTypeScriptFileCodeLine: byFileCodeLine.onlyTypeScript,
      onlySurgeTsFileCodeLine: byFileCodeLine.onlySurgeTs,
    },
    messageParity,
    summary: {
      byCodeMatch: byCode.onlyTypeScript.length === 0 && byCode.onlySurgeTs.length === 0,
      byFileCodeMatch:
        byFileCode.onlyTypeScript.length === 0 && byFileCode.onlySurgeTs.length === 0,
      byFileCodeLineMatch:
        byFileCodeLine.matches.length > 0 ||
        byFileCodeLine.onlyTypeScript.length > 0 ||
        byFileCodeLine.onlySurgeTs.length > 0
          ? byFileCodeLine.onlyTypeScript.length === 0 &&
            byFileCodeLine.onlySurgeTs.length === 0
          : null,
      messageMatch:
        messageParity.comparedLocations === 0 ? null : messageParity.mismatches.length === 0,
    },
    details: {
      onlyTypeScript: {
        rawDiagnosticFingerprints: groupDiagnosticsByFingerprint(onlyTypeScriptDiagnostics),
      },
      onlySurgeTs: {
        rawDiagnosticFingerprints: groupDiagnosticsByFingerprint(onlySurgeTsDiagnostics),
        rawTs2305ModuleExports: groupDiagnosticsByModuleExportExtractor(
          onlySurgeTsDiagnostics.filter((diagnostic) => diagnostic.code === 'TS2305'),
          (diagnostic) => {
            const exportInfo = extractTs2305ModuleExport(diagnostic.message);
            return exportInfo ? { moduleSpecifier: exportInfo.moduleSpecifier, exportName: exportInfo.exportName } : null;
          },
        ),
        rawTs2307ModuleSpecifiers: groupDiagnosticsByExtractor(
          onlySurgeTsDiagnostics.filter((diagnostic) => diagnostic.code === 'TS2307'),
          (diagnostic) => extractTs2307ModuleSpecifier(diagnostic.message),
        ),
        rawTs2304Identifiers: groupDiagnosticsByExtractor(
          onlySurgeTsDiagnostics.filter((diagnostic) => diagnostic.code === 'TS2304'),
          (diagnostic) => extractTs2304Identifier(diagnostic.message),
        ),
      },
    },
  };
}

/**
 * Pairs diagnostics that share an exact location and code (fileName, code,
 * line, column) and reports where only the message text differs. Span-level
 * differences are deliberately left to the byFileCodeLine dimension: a pair is
 * only message-compared when its location matches exactly, so the output
 * isolates pure message-text drift (literal vs widened types, alias names,
 * quoting) from span defects.
 */
export function compareMessages(
  typescript: NormalizedDiagnostic[],
  surgeTs: NormalizedDiagnostic[],
): MessageParity {
  const typeScriptByLocation = groupByLocation(typescript);
  const surgeTsByLocation = groupByLocation(surgeTs);

  let comparedLocations = 0;
  let matches = 0;
  const mismatches: MessageMismatch[] = [];

  const sharedKeys = [...typeScriptByLocation.keys()]
    .filter((key) => surgeTsByLocation.has(key))
    .sort((leftKey, rightKey) => leftKey.localeCompare(rightKey));

  for (const key of sharedKeys) {
    comparedLocations += 1;
    const typeScriptList = typeScriptByLocation.get(key) ?? [];
    const surgeTsList = surgeTsByLocation.get(key) ?? [];

    const surgeTsRemaining = new Map<string, number>();
    for (const diagnostic of surgeTsList) {
      const message = diagnostic.message ?? '';
      surgeTsRemaining.set(message, (surgeTsRemaining.get(message) ?? 0) + 1);
    }

    // Consume exact message matches first; whatever is left on each side is a
    // genuine text difference at the same location.
    const typeScriptUnmatched: string[] = [];
    for (const diagnostic of typeScriptList) {
      const message = diagnostic.message ?? '';
      const remaining = surgeTsRemaining.get(message) ?? 0;
      if (remaining > 0) {
        surgeTsRemaining.set(message, remaining - 1);
        matches += 1;
      } else {
        typeScriptUnmatched.push(message);
      }
    }

    const surgeTsUnmatched: string[] = [];
    for (const [message, count] of surgeTsRemaining) {
      for (let index = 0; index < count; index += 1) {
        surgeTsUnmatched.push(message);
      }
    }

    typeScriptUnmatched.sort((left, right) => left.localeCompare(right));
    surgeTsUnmatched.sort((left, right) => left.localeCompare(right));

    const representative = typeScriptList[0] ?? surgeTsList[0];
    const pairCount = Math.min(typeScriptUnmatched.length, surgeTsUnmatched.length);
    for (let index = 0; index < pairCount; index += 1) {
      mismatches.push({
        fileName: representative.fileName,
        code: representative.code,
        line: representative.line ?? null,
        column: representative.column ?? null,
        typescript: typeScriptUnmatched[index],
        surgeTs: surgeTsUnmatched[index],
      });
    }
  }

  mismatches.sort(
    (left, right) =>
      left.fileName.localeCompare(right.fileName) ||
      (left.line ?? -1) - (right.line ?? -1) ||
      (left.column ?? -1) - (right.column ?? -1) ||
      left.code.localeCompare(right.code),
  );

  return { comparedLocations, matches, mismatches };
}

function groupByLocation(diagnostics: NormalizedDiagnostic[]): Map<string, NormalizedDiagnostic[]> {
  const byLocation = new Map<string, NormalizedDiagnostic[]>();
  for (const diagnostic of diagnostics) {
    const key = keyByFullLocation(diagnostic);
    const existing = byLocation.get(key);
    if (existing) {
      existing.push(diagnostic);
    } else {
      byLocation.set(key, [diagnostic]);
    }
  }

  return byLocation;
}

export function keyByFullLocation(diagnostic: NormalizedDiagnostic): string {
  return `${diagnostic.fileName} :: ${diagnostic.code} :: line=${diagnostic.line ?? 'n/a'} :: column=${diagnostic.column ?? 'n/a'}`;
}

export function buildTypeScriptCommand(mode: 'project' | 'file', targetDisplay: string, ignoreConfig?: boolean): string {
  const bin = `node node_modules/${oracleTypeScript}/bin/tsc`;
  if (mode === 'project') {
    return `${bin} --noEmit --pretty false --project ${targetDisplay}`;
  }

  return ignoreConfig ? `${bin} --noEmit --pretty false --ignoreConfig ${targetDisplay}` : `${bin} --noEmit --pretty false ${targetDisplay}`;
}

export function buildSurgeTsCommand(
  mode: 'project' | 'file',
  targetDisplay: string,
  ignoreConfig?: boolean,
  stubExternalModules?: boolean,
  rustJobs?: number,
): string {
  const cargoToml = normalizePathForDisplay(path.join(workspaceRoot, 'Cargo.toml'));
  let args = `cargo run -q --manifest-path ${cargoToml} -p surge-ts-cli --`;

  if (mode === 'project') {
    args += ` --project ${targetDisplay} --format json`;
    if (stubExternalModules) {
      args += ` --stubExternalModules`;
    }
    if (rustJobs !== undefined) {
      args += ` --jobs ${rustJobs}`;
    }
    return args;
  }

  args += ` --format json`;
  if (ignoreConfig) {
    args += ` --ignoreConfig`;
  }
  if (stubExternalModules) {
    args += ` --stubExternalModules`;
  }
  args += ` ${targetDisplay}`;

  return args;
}

export function summarizeDiagnostics(diagnostics: NormalizedDiagnostic[]): DiagnosticTotals {
  return {
    total: diagnostics.length,
    byCode: countEntriesFromCounts(countDiagnostics(diagnostics, keyByCode)),
    byFileCode: countEntriesFromCounts(countDiagnostics(diagnostics, keyByFileCode)),
    byFileCodeLine: countEntriesFromCounts(
      countDiagnostics(diagnostics.filter(hasLineInfo), keyByFileCodeLine),
    ),
  };
}

export function compareBuckets(
  left: NormalizedDiagnostic[],
  right: NormalizedDiagnostic[],
  keyFn: (diagnostic: NormalizedDiagnostic) => string,
): {
  matches: CountBucket[];
  onlyTypeScript: CountBucket[];
  onlySurgeTs: CountBucket[];
} {
  const leftCounts = countDiagnostics(left, keyFn);
  const rightCounts = countDiagnostics(right, keyFn);
  const keys = new Set([...leftCounts.keys(), ...rightCounts.keys()]);
  const sortedKeys = [...keys].sort((leftKey, rightKey) => leftKey.localeCompare(rightKey));
  const matches: CountBucket[] = [];
  const onlyTypeScript: CountBucket[] = [];
  const onlySurgeTs: CountBucket[] = [];

  for (const key of sortedKeys) {
    const leftCount = leftCounts.get(key) ?? 0;
    const rightCount = rightCounts.get(key) ?? 0;
    if (leftCount === rightCount) {
      if (leftCount > 0) {
        matches.push({ key, typescript: leftCount, surgeTs: rightCount });
      }
      continue;
    }

    if (leftCount > 0 && rightCount === 0) {
      onlyTypeScript.push({ key, typescript: leftCount, surgeTs: 0 });
      continue;
    }

    if (rightCount > 0 && leftCount === 0) {
      onlySurgeTs.push({ key, typescript: 0, surgeTs: rightCount });
      continue;
    }

    if (leftCount > rightCount) {
      onlyTypeScript.push({ key, typescript: leftCount, surgeTs: rightCount });
    } else {
      onlySurgeTs.push({ key, typescript: leftCount, surgeTs: rightCount });
    }
  }

  return { matches, onlyTypeScript, onlySurgeTs };
}

export function countDiagnostics(
  diagnostics: NormalizedDiagnostic[],
  keyFn: (diagnostic: NormalizedDiagnostic) => string,
): Map<string, number> {
  const counts = new Map<string, number>();

  for (const diagnostic of diagnostics) {
    const key = keyFn(diagnostic);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }

  return counts;
}

export function countEntriesFromCounts(counts: Map<string, number>): CountEntry[] {
  return [...counts.entries()]
    .map(([key, count]) => ({ key, count }))
    .sort((left, right) => left.key.localeCompare(right.key));
}

export function subtractDiagnosticsByKey(
  left: NormalizedDiagnostic[],
  right: NormalizedDiagnostic[],
  keyFn: (diagnostic: NormalizedDiagnostic) => string,
): {
  onlyLeft: NormalizedDiagnostic[];
  onlyRight: NormalizedDiagnostic[];
} {
  const leftRemaining = countDiagnostics(left, keyFn);
  const rightRemaining = countDiagnostics(right, keyFn);
  const onlyLeft: NormalizedDiagnostic[] = [];
  const onlyRight: NormalizedDiagnostic[] = [];

  for (const diagnostic of left) {
    const key = keyFn(diagnostic);
    const remaining = rightRemaining.get(key) ?? 0;
    if (remaining > 0) {
      rightRemaining.set(key, remaining - 1);
    } else {
      onlyLeft.push(diagnostic);
    }
  }

  for (const diagnostic of right) {
    const key = keyFn(diagnostic);
    const remaining = leftRemaining.get(key) ?? 0;
    if (remaining > 0) {
      leftRemaining.set(key, remaining - 1);
    } else {
      onlyRight.push(diagnostic);
    }
  }

  return { onlyLeft, onlyRight };
}

export function groupDiagnosticsByExtractor(
  diagnostics: NormalizedDiagnostic[],
  extractor: (diagnostic: NormalizedDiagnostic) => string | null,
): CountEntry[] {
  const counts = new Map<string, number>();

  for (const diagnostic of diagnostics) {
    const key = extractor(diagnostic);
    if (!key) {
      continue;
    }

    counts.set(key, (counts.get(key) ?? 0) + 1);
  }

  return countEntriesFromCounts(counts);
}

export function groupDiagnosticsByModuleExportExtractor(
  diagnostics: NormalizedDiagnostic[],
  extractor: (diagnostic: NormalizedDiagnostic) => { moduleSpecifier: string; exportName: string } | null,
): ModuleExportCountEntry[] {
  const counts = new Map<string, ModuleExportCountEntry>();

  for (const diagnostic of diagnostics) {
    const bucket = extractor(diagnostic);
    if (!bucket) {
      continue;
    }

    const dedupeKey = `${bucket.moduleSpecifier} :: ${bucket.exportName}`;
    const existing = counts.get(dedupeKey);
    if (existing) {
      existing.count += 1;
      continue;
    }

    counts.set(dedupeKey, { ...bucket, count: 1 });
  }

  return [...counts.values()].sort(
    (left, right) =>
      right.count - left.count || left.moduleSpecifier.localeCompare(right.moduleSpecifier) || left.exportName.localeCompare(right.exportName),
  );
}

export function groupDiagnosticsByFingerprint(
  diagnostics: NormalizedDiagnostic[],
): DiagnosticFingerprintCountEntry[] {
  const counts = new Map<string, DiagnosticFingerprintCountEntry>();

  for (const diagnostic of diagnostics) {
    const fingerprint = normalizeDiagnostic(diagnostic);
    const dedupeKey = keyByDiagnosticFingerprint(diagnostic);
    const existing = counts.get(dedupeKey);
    if (existing) {
      existing.count += 1;
      continue;
    }

    counts.set(dedupeKey, { ...fingerprint, count: 1 });
  }

  return [...counts.values()].sort(
    (left, right) =>
      right.count - left.count ||
      left.fileName.localeCompare(right.fileName) ||
      (left.line ?? -1) - (right.line ?? -1) ||
      (left.column ?? -1) - (right.column ?? -1) ||
      left.code.localeCompare(right.code) ||
      (left.message ?? '').localeCompare(right.message ?? ''),
  );
}

export function extractTs2307ModuleSpecifier(message?: string): string | null {
  if (!message) {
    return null;
  }

  const match = message.match(/module ['"]([^'"]+)['"]/i);
  return match ? match[1] : null;
}

export function extractTs2305ModuleExport(
  message?: string,
): { moduleSpecifier: string; exportName: string } | null {
  if (!message) {
    return null;
  }

  const match = message.match(
    /Module ['"]([^'"]+)['"] has no exported member ['"]([^'"]+)['"]/i,
  );
  return match ? { moduleSpecifier: match[1], exportName: match[2] } : null;
}

export function extractTs2304Identifier(message?: string): string | null {
  if (!message) {
    return null;
  }

  const match = message.match(/Cannot find (?:name|namespace) ['"]([^'"]+)['"]/i);
  return match ? match[1] : null;
}

export function keyByCode(diagnostic: NormalizedDiagnostic): string {
  return diagnostic.code;
}

export function keyByFileCode(diagnostic: NormalizedDiagnostic): string {
  return `${diagnostic.fileName} :: ${diagnostic.code}`;
}

export function keyByFileCodeLine(diagnostic: NormalizedDiagnostic): string {
  return `${diagnostic.fileName} :: ${diagnostic.code} :: line=${diagnostic.line ?? 0}`;
}

export function keyByDiagnosticFingerprint(diagnostic: NormalizedDiagnostic): string {
  return JSON.stringify(normalizeDiagnostic(diagnostic));
}

export function hasLineInfo(diagnostic: NormalizedDiagnostic): boolean {
  return typeof diagnostic.line === 'number' && typeof diagnostic.column === 'number';
}

export function formatDiagnosticFingerprintEntry(entry: DiagnosticFingerprintCountEntry): string {
  const location = `${entry.fileName}:${entry.line ?? 'n/a'}:${entry.column ?? 'n/a'}`;
  const message = entry.message ?? '(no message)';
  return `${location} ${entry.code} ${entry.count} ${message}`;
}

export function renderComparisonText(comparison: ComparisonResult): string {
  const lines: string[] = [];
  lines.push('TypeScript oracle comparison');
  lines.push(`Mode: ${comparison.mode}`);
  lines.push(comparison.mode === 'project' ? `Project: ${comparison.project}` : `File: ${comparison.file}`);
  lines.push('');

  if (comparison.surgeTsOptions?.stubExternalModules) {
    lines.push('surge-ts options: --stubExternalModules');
    lines.push('Note: --stubExternalModules is a surge-ts-only compatibility mode.');
    lines.push('');
  }

  lines.push('Tooling:');
  lines.push(`TypeScript version: ${comparison.tooling.typescriptVersion}`);
  lines.push(`TypeScript command: ${comparison.tooling.typescriptCommand}`);
  lines.push(`surge-ts command: ${comparison.tooling.surgeTsCommand}`);
  if (comparison.tooling.surgeTsJobs !== undefined) {
    lines.push(`surge-ts jobs: ${comparison.tooling.surgeTsJobs}`);
  }
  lines.push('');
  lines.push('Totals:');
  lines.push(`TypeScript diagnostics: ${comparison.typescript.total}`);
  lines.push(`surge-ts diagnostics: ${comparison.surgeTs.total}`);
  lines.push('');
  if (comparison.warnings && comparison.warnings.length > 0) {
    lines.push('Warnings:');
    for (const warning of comparison.warnings) {
      lines.push(`  ${warning}`);
    }
    lines.push('');
  }
  lines.push('Summary:');
  lines.push(`  Code-count match: ${comparison.summary.byCodeMatch ? 'yes' : 'no'}`);
  lines.push(`  File/code match: ${comparison.summary.byFileCodeMatch ? 'yes' : 'no'}`);
  lines.push(
    `  File/code/line match: ${
      comparison.summary.byFileCodeLineMatch === null
        ? 'n/a'
        : comparison.summary.byFileCodeLineMatch
          ? 'yes'
          : 'no'
    }`,
  );
  lines.push(
    `  Message match: ${
      comparison.summary.messageMatch === null
        ? 'n/a'
        : comparison.summary.messageMatch
          ? 'yes'
          : 'no'
    }`,
  );
  lines.push('');
  appendMessageParitySection(lines, comparison.messageParity);
  lines.push('Raw message extraction, not root-cause classification:');
  if (comparison.details?.onlySurgeTs?.rawTs2305ModuleExports?.length) {
    lines.push('  TS2305 module/export:');
    for (const entry of comparison.details.onlySurgeTs.rawTs2305ModuleExports.slice(0, 10)) {
      lines.push(`    ${entry.moduleSpecifier} :: ${entry.exportName}  ${entry.count}`);
    }
  }
  if (comparison.details?.onlySurgeTs?.rawTs2307ModuleSpecifiers?.length) {
    lines.push('  TS2307 specifiers:');
    for (const entry of comparison.details.onlySurgeTs.rawTs2307ModuleSpecifiers.slice(0, 10)) {
      lines.push(`    ${entry.key}  ${entry.count}`);
    }
  }
  if (comparison.details?.onlySurgeTs?.rawTs2304Identifiers?.length) {
    lines.push('  TS2304 identifiers:');
    for (const entry of comparison.details.onlySurgeTs.rawTs2304Identifiers.slice(0, 10)) {
      lines.push(`    ${entry.key}  ${entry.count}`);
    }
  }
  lines.push('');
  if (comparison.details?.onlySurgeTs?.rawDiagnosticFingerprints?.length) {
    lines.push('Top ONLY_RUST raw diagnostic fingerprints:');
    for (const entry of comparison.details.onlySurgeTs.rawDiagnosticFingerprints.slice(0, 10)) {
      lines.push(`  ${formatDiagnosticFingerprintEntry(entry)}`);
    }
    lines.push('');
  }
  if (comparison.details?.onlyTypeScript?.rawDiagnosticFingerprints?.length) {
    lines.push('Top ONLY_TS raw diagnostic fingerprints:');
    for (const entry of comparison.details.onlyTypeScript.rawDiagnosticFingerprints.slice(0, 10)) {
      lines.push(`  ${formatDiagnosticFingerprintEntry(entry)}`);
    }
    lines.push('');
  }
  lines.push('By code:');
  appendBucketSection(
    lines,
    comparison.matches.byCode,
    comparison.matches.onlyTypeScript,
    comparison.matches.onlySurgeTs,
  );
  lines.push('');
  lines.push('By file/code:');
  appendBucketSection(
    lines,
    comparison.matches.byFileCode,
    comparison.matches.onlyTypeScriptFileCode,
    comparison.matches.onlySurgeTsFileCode,
  );
  lines.push('');
  lines.push('By file/code/line:');
  if (comparison.summary.byFileCodeLineMatch === null) {
    lines.push('  (no line information on both sides)');
  } else {
    appendBucketSection(
      lines,
      comparison.matches.byFileCodeLine,
      comparison.matches.onlyTypeScriptFileCodeLine,
      comparison.matches.onlySurgeTsFileCodeLine,
    );
  }
  return `${lines.join('\n')}\n`;
}

function buildComparisonWarnings(
  typescript: NormalizedDiagnostic[],
  surgeTs: NormalizedDiagnostic[],
): string[] {
  const warnings: string[] = [];
  const rustOnlyDiagnostics = surgeTs.filter((diagnostic) =>
    diagnostic.code.startsWith('surge::'),
  );

  if (rustOnlyDiagnostics.length > 0) {
    warnings.push(
      `Rust-only surge::* diagnostics in tsc profile: ${rustOnlyDiagnostics.length}`,
    );
  }

  if (surgeTs.length > typescript.length * 2) {
    warnings.push(
      `Severe over-report: surge-ts diagnostics (${surgeTs.length}) exceed TypeScript diagnostics (${typescript.length}) by more than 2x`,
    );
  }

  return warnings;
}

export function appendMessageParitySection(lines: string[], messageParity: MessageParity): void {
  lines.push('Message parity (same file/code/line/column, message text differs):');
  if (messageParity.comparedLocations === 0) {
    lines.push('  (no diagnostics share an exact location on both sides)');
    lines.push('');
    return;
  }

  lines.push(
    `  Compared locations: ${messageParity.comparedLocations}  matches: ${messageParity.matches}  mismatches: ${messageParity.mismatches.length}`,
  );
  for (const mismatch of messageParity.mismatches.slice(0, 20)) {
    const location = `${mismatch.fileName}:${mismatch.line ?? 'n/a'}:${mismatch.column ?? 'n/a'}`;
    lines.push(`  ${location} ${mismatch.code}`);
    lines.push(`    tsc : ${mismatch.typescript}`);
    lines.push(`    rust: ${mismatch.surgeTs}`);
  }
  if (messageParity.mismatches.length > 20) {
    lines.push(`  ... and ${messageParity.mismatches.length - 20} more`);
  }
  lines.push('');
}

export function appendBucketSection(
  lines: string[],
  matches: CountBucket[],
  onlyTypeScript: CountBucket[],
  onlySurgeTs: CountBucket[],
): void {
  if (matches.length === 0 && onlyTypeScript.length === 0 && onlySurgeTs.length === 0) {
    lines.push('  (none)');
    return;
  }

  for (const bucket of matches) {
    lines.push(`MATCH ${formatBucketKey(bucket.key)} ${bucket.typescript}`);
  }

  for (const bucket of onlyTypeScript) {
    if (bucket.surgeTs === 0) {
      lines.push(`ONLY_TS ${formatBucketKey(bucket.key)} ${bucket.typescript}`);
    } else {
      lines.push(
        `DIFF ${formatBucketKey(bucket.key)} TypeScript=${bucket.typescript} surge-ts=${bucket.surgeTs}`,
      );
    }
  }

  for (const bucket of onlySurgeTs) {
    if (bucket.typescript === 0) {
      lines.push(`ONLY_RUST ${formatBucketKey(bucket.key)} ${bucket.surgeTs}`);
    } else {
      lines.push(
        `DIFF ${formatBucketKey(bucket.key)} TypeScript=${bucket.typescript} surge-ts=${bucket.surgeTs}`,
      );
    }
  }
}

export function formatBucketKey(key: string): string {
  const parts = key.split(' :: ');
  if (parts.length === 1) {
    return parts[0];
  }
  if (parts.length === 2) {
    return `${parts[0]} ${parts[1]}`;
  }
  return `${parts[0]} ${parts[1]} ${parts[2]}`;
}

function printHelpAndExit(): never {
  process.stdout.write(
    [
      'Usage:',
      '  pnpm run oracle:compare -- --project <tsconfig.json|preset>',
      '  pnpm run oracle:compare -- --file <source.ts>',
      '',
      'Options:',
      '  --project <path|preset>   Compare a tsconfig file or known fixture preset.',
      '  --file <path>             Compare a single TypeScript source file.',
      '  --fixture <preset>        Alias for --project when passing a preset name.',
      '  --maxDiagnostics <n>      Limit diagnostics on both sides before comparing.',
      '  --json                    Emit machine-readable comparison output.',
      '  --failOnMismatch          Exit with code 1 when code/file mismatches exist.',
      '  --strictCodes             Alias for --failOnMismatch.',
      '  --strictMessages          Exit with code 1 when any same-location message text differs from tsc.',
      '  --rustJobs <n>            Pass a deterministic project-checking job count to surge-ts.',
      '',
      'Known presets:',
      `  ${Object.keys(fixturePresets).join(', ')}`,
      '',
      'Project mode examples:',
      '  pnpm run oracle:compare -- --project generics-basic',
      '  pnpm run oracle:compare -- --project tests/compat-projects/generics-basic/tsconfig.json',
      '',
      'File mode examples:',
      '  pnpm run oracle:compare -- --file examples/basic.ts',
      '  pnpm run oracle:compare -- --file examples/assignment.ts',
      '',
    ].join('\n'),
  );
  process.exit(0);
}

function formatParseFailure(output: string, error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return [`Parse error: ${message}`, 'Output:', output.trim() || '(empty)'].join('\n');
}

// Report the version of the package the oracle actually invokes; a workspace
// whose node_modules lags the pinned alias would otherwise record a version it
// never ran. The devDependency spec is only a fallback for an uninstalled tree.
function readPinnedTypeScriptVersion(): string {
  const installedManifest = path.join(workspaceRoot, 'node_modules', oracleTypeScript, 'package.json');
  try {
    const manifest = JSON.parse(readFileSync(installedManifest, 'utf8')) as { version?: string };
    if (typeof manifest.version === 'string') {
      return manifest.version;
    }
  } catch {
    // fall through to the declared spec
  }

  const packageJsonPath = path.join(workspaceRoot, 'package.json');
  const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8')) as {
    devDependencies?: Record<string, string>;
  };

  const deps = packageJson.devDependencies ?? {};
  const spec = deps[oracleTypeScript] ?? 'unknown';
  // devDependency specs are pnpm aliases (`npm:typescript@7.0.2`); surface just
  // the declared version for display.
  return spec.replace(/^npm:typescript@/, '');
}

function looksLikePath(value: string): boolean {
  return value.includes('/') || value.includes('\\') || value.endsWith('.json') || value.startsWith('.');
}

export function isSourceFilePath(value: string): boolean {
  return ['.ts', '.tsx', '.js', '.mts', '.cts'].includes(path.extname(value).toLowerCase());
}

export function isTsConfigPath(value: string): boolean {
  return path.basename(normalizePathForDisplay(value)).toLowerCase() === 'tsconfig.json';
}

export function isTsConfigVariantPath(value: string): boolean {
  const basename = path.basename(normalizePathForDisplay(value)).toLowerCase();
  return basename.endsWith('.json') && basename.includes('tsconfig');
}

export function resolveWorkspacePath(value: string): string {
  return path.isAbsolute(value) ? value : path.resolve(workspaceRoot, value);
}

function isAbsolutePathLike(value: string): boolean {
  const normalized = normalizePathForDisplay(value);
  return (
    path.isAbsolute(value) ||
    path.win32.isAbsolute(value) ||
    normalized.startsWith('/') ||
    /^[A-Za-z]:\//.test(normalized) ||
    normalized.startsWith('//')
  );
}

function executeComparison(
  mode: OracleMode,
  maxDiagnostics?: number,
): ComparisonResult {
  const comparisonPath = mode.kind === 'project' ? mode.resolvedTsconfig : mode.resolvedFile;
  const comparisonDisplay = displayComparisonTargetPath(comparisonPath);
  const projectDir = path.dirname(comparisonPath);
  const tsc = runTsc(mode);
  const rust = runSurgeTs(mode, maxDiagnostics, mode.kind === 'project' ? mode.rustJobs : undefined);
  const rustOutput = rust.stdout.trim() ? rust.stdout : rust.stderr;

  const tscDiagnostics = limitDiagnostics(
    parseTypeScriptDiagnostics(`${tsc.stdout}${tsc.stderr}`, projectDir),
    maxDiagnostics,
  );
  const rustDiagnostics = limitDiagnostics(parseSurgeTsDiagnostics(rustOutput, projectDir), maxDiagnostics);

  return compareDiagnostics(
    mode.kind,
    comparisonDisplay,
    tscDiagnostics,
    rustDiagnostics,
    mode.ignoreConfig,
    mode.stubExternalModules,
    mode.kind === 'project' ? mode.rustJobs : undefined,
  );
}

export function displayComparisonTargetPath(targetPath: string): string {
  const relative = path.relative(workspaceRoot, targetPath);
  return relative.startsWith('..') ? normalizePathForDisplay(targetPath) : normalizePathForDisplay(relative);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  try {
    main();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${message}\n`);
    process.exit(1);
  }
}
