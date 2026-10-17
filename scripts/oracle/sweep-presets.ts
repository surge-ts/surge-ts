#!/usr/bin/env tsx

import { spawn } from 'node:child_process';
import { readdirSync, statSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  displayComparisonTargetPath,
  fixturePresets,
  renderComparisonText,
  resolveFilePath,
  resolveProjectPresetOrPath,
  resolveSurgeBin,
  resolveWorkspacePath,
} from './compare-tsc';
import type { ComparisonResult } from './compare-tsc';

const scriptPath = fileURLToPath(import.meta.url);
const scriptDir = path.dirname(scriptPath);
const workspaceRoot = path.resolve(scriptDir, '../..');
const compareScript = path.join(scriptDir, 'compare-tsc.ts');
// Spawn the local tsx binary directly instead of `pnpm exec tsx`: pnpm reruns
// its verify-deps-before-run check (lockfile vs node_modules across the whole
// workspace) on every invocation, costing ~10s per child here.
// tsx's CLI run by this Node: Windows refuses to spawn the `.cmd` shim
// without a shell (EINVAL).
const tsxCli = path.join(workspaceRoot, 'node_modules', 'tsx', 'dist', 'cli.mjs');

export type TargetKind = 'preset' | 'project' | 'file';

export type SweepTarget = {
  name: string;
  kind: TargetKind;
  value: string;
  resolvedPath: string;
};

export type SweepArgs = {
  all: boolean;
  filters: string[];
  excludes: string[];
  projects: string[];
  files: string[];
  discover: string[];
  list: boolean;
  json: boolean;
  verbose: boolean;
  strictMessages: boolean;
  strictSpans: boolean;
  maxDiagnostics?: number;
  jobs?: number;
};

export type PresetResult = {
  preset: string;
  kind: TargetKind;
  passed: boolean;
  typescriptDiagnostics: number;
  rustDiagnostics: number;
  onlyTsc: number;
  onlyRust: number;
  fileCodeLineMatch: boolean;
  codeCountMatch: boolean;
  messageMatch: boolean | null;
  spanMatch: boolean;
  elapsedMs: number;
  error?: string;
};

export type SweepSummary = {
  total: number;
  passed: number;
  failed: number;
  skipped: number;
  typescriptDiagnostics: number;
  rustDiagnostics: number;
  onlyTsc: number;
  onlyRust: number;
  fileCodeLineMismatches: number;
  codeCountMismatches: number;
  messageDriftOnly: number;
  spanDriftOnly: number;
  elapsedMs: number;
  exitCode: number;
};

export type Selection = {
  selected: SweepTarget[];
  skipped: SweepTarget[];
  hasCriteria: boolean;
};

type RunOutcome = {
  result: PresetResult;
  comparison?: ComparisonResult;
};

export function listPresetNames(): string[] {
  return Object.keys(fixturePresets);
}

export function presetTargets(): SweepTarget[] {
  return Object.entries(fixturePresets).map(([name, resolvedPath]) => ({
    name,
    kind: 'preset',
    value: name,
    resolvedPath,
  }));
}

export function makeProjectTarget(value: string): SweepTarget {
  const resolvedPath = resolveProjectPresetOrPath(value);
  const name = fixturePresets[value] ? value : displayComparisonTargetPath(resolvedPath);
  return { name, kind: 'project', value, resolvedPath };
}

export function makeFileTarget(value: string): SweepTarget {
  const resolvedPath = resolveFilePath(value);
  return { name: displayComparisonTargetPath(resolvedPath), kind: 'file', value, resolvedPath };
}

export function discoverProjectTargets(dir: string): SweepTarget[] {
  const root = resolveWorkspacePath(dir);
  if (!statSync(root, { throwIfNoEntry: false })?.isDirectory()) {
    throw new Error(`--discover expects an existing directory: ${dir}`);
  }

  const found: SweepTarget[] = [];
  const walk = (current: string): void => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        if (entry.name === 'node_modules' || entry.name.startsWith('.')) {
          continue;
        }
        walk(path.join(current, entry.name));
      } else if (entry.isFile() && entry.name === 'tsconfig.json') {
        const resolvedPath = path.join(current, entry.name);
        found.push({
          name: displayComparisonTargetPath(resolvedPath),
          kind: 'project',
          value: resolvedPath,
          resolvedPath,
        });
      }
    }
  };

  walk(root);
  return found.sort((left, right) => left.name.localeCompare(right.name));
}

export function dedupeTargets(targets: SweepTarget[]): SweepTarget[] {
  const seen = new Set<string>();
  const result: SweepTarget[] = [];
  for (const target of targets) {
    if (seen.has(target.resolvedPath)) {
      continue;
    }
    seen.add(target.resolvedPath);
    result.push(target);
  }
  return result;
}

export function parseSweepArgs(argv: string[]): SweepArgs {
  const parsed: SweepArgs = {
    all: false,
    filters: [],
    excludes: [],
    projects: [],
    files: [],
    discover: [],
    list: false,
    json: false,
    verbose: false,
    strictMessages: false,
    strictSpans: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];

    if (arg === '--help' || arg === '-h') {
      printHelpAndExit();
    } else if (arg === '--') {
      continue;
    } else if (arg === '--all') {
      parsed.all = true;
    } else if (arg === '--filter') {
      parsed.filters.push(requireValue(arg, argv[++index]));
    } else if (arg === '--exclude') {
      parsed.excludes.push(requireValue(arg, argv[++index]));
    } else if (arg === '--project' || arg === '--fixture') {
      parsed.projects.push(requireValue(arg, argv[++index]));
    } else if (arg === '--file') {
      parsed.files.push(requireValue(arg, argv[++index]));
    } else if (arg === '--discover') {
      parsed.discover.push(requireValue(arg, argv[++index]));
    } else if (arg === '--list') {
      parsed.list = true;
    } else if (arg === '--json') {
      parsed.json = true;
    } else if (arg === '--verbose') {
      parsed.verbose = true;
    } else if (arg === '--strictMessages') {
      parsed.strictMessages = true;
    } else if (arg === '--strictSpans') {
      parsed.strictSpans = true;
    } else if (arg === '--maxDiagnostics') {
      parsed.maxDiagnostics = parsePositiveInteger(arg, argv[++index]);
    } else if (arg === '--jobs') {
      parsed.jobs = parsePositiveInteger(arg, argv[++index]);
    } else if (arg.startsWith('--')) {
      throw new Error(`unknown argument: ${arg}`);
    } else {
      throw new Error(
        `unexpected positional argument: ${arg}. Use --all, --filter <substring>, --project <path>, --file <path>, or --discover <dir>.`,
      );
    }
  }

  return parsed;
}

function requireValue(flag: string, value: string | undefined): string {
  if (!value) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

function parsePositiveInteger(flag: string, value: string | undefined): number {
  const parsedValue = Number(requireValue(flag, value));
  if (!Number.isInteger(parsedValue) || parsedValue <= 0) {
    throw new Error(`${flag} must be a positive integer`);
  }
  return parsedValue;
}

function matchesAny(name: string, substrings: string[]): boolean {
  return substrings.some((substring) => name.includes(substring));
}

export function selectTargets(
  args: SweepArgs,
  presets: SweepTarget[],
  explicit: SweepTarget[],
  discovered: SweepTarget[],
): Selection {
  const hasFilter = args.filters.length > 0;
  const hasExplicitSources =
    args.projects.length > 0 || args.files.length > 0 || args.discover.length > 0;
  const hasCriteria = args.all || hasFilter || args.list || hasExplicitSources;

  // Bare `--list` (no other selection signal) is a convenience that lists every
  // preset; once a filter or an explicit source is present, `--list` only
  // reflects those and does not pull in the whole registry.
  const listAllShortcut = args.list && !hasFilter && !hasExplicitSources;

  let selectedPresets: SweepTarget[];
  if (hasFilter) {
    selectedPresets = presets.filter((target) => matchesAny(target.name, args.filters));
  } else if (args.all || listAllShortcut) {
    selectedPresets = presets;
  } else {
    selectedPresets = [];
  }

  const keptDiscovered = hasFilter
    ? discovered.filter((target) => matchesAny(target.name, args.filters))
    : discovered;

  const combined = dedupeTargets([...selectedPresets, ...explicit, ...keptDiscovered]);
  const selected = combined.filter((target) => !matchesAny(target.name, args.excludes));
  const skipped = combined.filter((target) => matchesAny(target.name, args.excludes));

  return { selected, skipped, hasCriteria };
}

export function deriveSpanMatch(comparison: ComparisonResult): boolean {
  const onlyTs = comparison.details?.onlyTypeScript?.rawDiagnosticFingerprints ?? [];
  const onlyRust = comparison.details?.onlySurgeTs?.rawDiagnosticFingerprints ?? [];

  const rustColumnsByLine = new Map<string, Set<number>>();
  for (const entry of onlyRust) {
    if (entry.line === null || entry.column === null) {
      continue;
    }
    const key = `${entry.fileName} :: ${entry.code} :: ${entry.line}`;
    const columns = rustColumnsByLine.get(key) ?? new Set<number>();
    columns.add(entry.column);
    rustColumnsByLine.set(key, columns);
  }

  for (const entry of onlyTs) {
    if (entry.line === null || entry.column === null) {
      continue;
    }
    const key = `${entry.fileName} :: ${entry.code} :: ${entry.line}`;
    const columns = rustColumnsByLine.get(key);
    if (columns && !columns.has(entry.column)) {
      return false;
    }
  }

  return true;
}

function sumBucketSurplus(
  buckets: { typescript: number; surgeTs: number }[],
  pick: (bucket: { typescript: number; surgeTs: number }) => number,
): number {
  return buckets.reduce((total, bucket) => total + Math.max(0, pick(bucket)), 0);
}

export function deriveResult(
  target: SweepTarget,
  comparison: ComparisonResult,
  elapsedMs: number,
  args: SweepArgs,
): PresetResult {
  const codeCountMatch = comparison.summary.byCodeMatch;
  const fileCodeMatch = comparison.summary.byFileCodeMatch;
  const lineMatch = comparison.summary.byFileCodeLineMatch !== false;
  const fileCodeLineMatch = fileCodeMatch && lineMatch;
  const messageMatch = comparison.summary.messageMatch;
  const spanMatch = deriveSpanMatch(comparison);

  const gatePassed = codeCountMatch && fileCodeLineMatch;
  const passed =
    gatePassed &&
    (!args.strictMessages || messageMatch !== false) &&
    (!args.strictSpans || spanMatch);

  return {
    preset: target.name,
    kind: target.kind,
    passed,
    typescriptDiagnostics: comparison.typescript.total,
    rustDiagnostics: comparison.surgeTs.total,
    onlyTsc: sumBucketSurplus(comparison.matches.onlyTypeScript, (b) => b.typescript - b.surgeTs),
    onlyRust: sumBucketSurplus(comparison.matches.onlySurgeTs, (b) => b.surgeTs - b.typescript),
    fileCodeLineMatch,
    codeCountMatch,
    messageMatch,
    spanMatch,
    elapsedMs,
  };
}

function errorResult(target: SweepTarget, elapsedMs: number, message: string): PresetResult {
  return {
    preset: target.name,
    kind: target.kind,
    passed: false,
    typescriptDiagnostics: 0,
    rustDiagnostics: 0,
    onlyTsc: 0,
    onlyRust: 0,
    fileCodeLineMatch: false,
    codeCountMatch: false,
    messageMatch: null,
    spanMatch: false,
    elapsedMs,
    error: message,
  };
}

export function buildSummary(
  results: PresetResult[],
  skipped: SweepTarget[],
  elapsedMs: number,
): SweepSummary {
  const passed = results.filter((result) => result.passed).length;
  const failed = results.length - passed;

  return {
    total: results.length,
    passed,
    failed,
    skipped: skipped.length,
    typescriptDiagnostics: results.reduce((total, result) => total + result.typescriptDiagnostics, 0),
    rustDiagnostics: results.reduce((total, result) => total + result.rustDiagnostics, 0),
    onlyTsc: results.reduce((total, result) => total + result.onlyTsc, 0),
    onlyRust: results.reduce((total, result) => total + result.onlyRust, 0),
    fileCodeLineMismatches: results.filter((result) => !result.fileCodeLineMatch).length,
    codeCountMismatches: results.filter((result) => !result.codeCountMatch).length,
    messageDriftOnly: results.filter((result) => result.passed && result.messageMatch === false).length,
    spanDriftOnly: results.filter((result) => result.passed && !result.spanMatch).length,
    elapsedMs,
    exitCode: failed > 0 ? 1 : 0,
  };
}

function matchLabel(value: boolean | null): string {
  if (value === null) {
    return 'na';
  }
  return value ? 'yes' : 'no';
}

export function formatPresetLine(result: PresetResult): string {
  const status = result.passed ? 'PASS' : 'FAIL';
  const fields = [
    `ts=${result.typescriptDiagnostics}`,
    `rust=${result.rustDiagnostics}`,
    `onlyTsc=${result.onlyTsc}`,
    `onlyRust=${result.onlyRust}`,
    `fileCodeLine=${matchLabel(result.fileCodeLineMatch)}`,
    `message=${matchLabel(result.messageMatch)}`,
    `span=${matchLabel(result.spanMatch)}`,
    `elapsed=${result.elapsedMs}ms`,
  ];
  const line = `${status} ${result.preset} ${fields.join(' ')}`;
  return result.error ? `${line} ERROR` : line;
}

export function formatSummaryText(summary: SweepSummary): string {
  return [
    'Oracle sweep summary',
    `selected: ${summary.total}`,
    `passed: ${summary.passed}`,
    `failed: ${summary.failed}`,
    `skipped: ${summary.skipped}`,
    `typescriptDiagnostics: ${summary.typescriptDiagnostics}`,
    `rustDiagnostics: ${summary.rustDiagnostics}`,
    `onlyTsc: ${summary.onlyTsc}`,
    `onlyRust: ${summary.onlyRust}`,
    `fileCodeLineMismatches: ${summary.fileCodeLineMismatches}`,
    `codeCountMismatches: ${summary.codeCountMismatches}`,
    `messageDriftOnly: ${summary.messageDriftOnly}`,
    `spanDriftOnly: ${summary.spanDriftOnly}`,
    `elapsed: ${(summary.elapsedMs / 1000).toFixed(1)}s`,
  ].join('\n');
}

function indent(text: string): string {
  return text
    .split('\n')
    .map((line) => (line ? `    ${line}` : line))
    .join('\n');
}

function formatFailureDetail(comparison: ComparisonResult): string {
  const lines: string[] = [];
  const pushBucket = (label: string, buckets: { key: string; typescript: number; surgeTs: number }[]) => {
    for (const bucket of buckets) {
      lines.push(`${label} ${bucket.key} (tsc=${bucket.typescript} rust=${bucket.surgeTs})`);
    }
  };

  pushBucket('ONLY_TSC', comparison.matches.onlyTypeScript);
  pushBucket('ONLY_RUST', comparison.matches.onlySurgeTs);
  pushBucket('FILE/CODE ONLY_TSC', comparison.matches.onlyTypeScriptFileCode);
  pushBucket('FILE/CODE ONLY_RUST', comparison.matches.onlySurgeTsFileCode);
  pushBucket('FILE/CODE/LINE ONLY_TSC', comparison.matches.onlyTypeScriptFileCodeLine);
  pushBucket('FILE/CODE/LINE ONLY_RUST', comparison.matches.onlySurgeTsFileCodeLine);

  return lines.length > 0 ? lines.join('\n') : '(no code/file/line buckets to report)';
}

function runTarget(target: SweepTarget, args: SweepArgs): Promise<RunOutcome> {
  return new Promise((resolve) => {
    const childArgs = [compareScript];
    if (target.kind === 'file') {
      childArgs.push('--file', target.value);
    } else {
      childArgs.push('--project', target.value);
      // Pin each child's surge run to a single thread so many sweep workers can
      // run concurrently without nested oversubscription (worker count × surge's
      // own job count). Diagnostics are independent of --rustJobs.
      childArgs.push('--rustJobs', '1');
    }
    childArgs.push('--json');
    if (args.maxDiagnostics !== undefined) {
      childArgs.push('--maxDiagnostics', String(args.maxDiagnostics));
    }

    const started = Date.now();
    const child = spawn(process.execPath, [tsxCli, ...childArgs], {
      cwd: workspaceRoot,
    });

    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });

    child.on('error', (error) => {
      resolve({ result: errorResult(target, Date.now() - started, error.message) });
    });

    child.on('close', () => {
      const elapsedMs = Date.now() - started;
      let comparison: ComparisonResult;
      try {
        comparison = JSON.parse(stdout) as ComparisonResult;
      } catch {
        const detail = (stderr.trim() || stdout.trim() || 'no output').split('\n').slice(0, 8).join('\n');
        resolve({ result: errorResult(target, elapsedMs, detail) });
        return;
      }
      resolve({ result: deriveResult(target, comparison, elapsedMs, args), comparison });
    });
  });
}

async function runSweep(selected: SweepTarget[], args: SweepArgs): Promise<RunOutcome[]> {
  const defaultJobs = os.cpus().length || 1;
  const jobs = Math.max(1, Math.min(args.jobs ?? defaultJobs, selected.length));
  const outcomes = new Array<RunOutcome>(selected.length);
  let nextIndex = 0;

  const worker = async (): Promise<void> => {
    while (true) {
      const index = nextIndex;
      nextIndex += 1;
      if (index >= selected.length) {
        return;
      }
      outcomes[index] = await runTarget(selected[index], args);
      if (!args.json || args.verbose) {
        process.stderr.write(`  ran ${selected[index].name}\n`);
      }
    }
  };

  await Promise.all(Array.from({ length: jobs }, () => worker()));
  return outcomes;
}

export async function main(argv = process.argv.slice(2)): Promise<number> {
  const args = parseSweepArgs(argv);

  const explicit = [...args.projects.map(makeProjectTarget), ...args.files.map(makeFileTarget)];
  const discovered = args.discover.flatMap(discoverProjectTargets);
  const selection = selectTargets(args, presetTargets(), explicit, discovered);

  if (!selection.hasCriteria) {
    process.stdout.write(usageText());
    return 0;
  }

  if (args.list) {
    if (args.json) {
      process.stdout.write(
        `${JSON.stringify(
          {
            selected: selection.selected.map((target) => target.name),
            skipped: selection.skipped.map((target) => target.name),
          },
          null,
          2,
        )}\n`,
      );
    } else {
      for (const target of selection.selected) {
        process.stdout.write(`${target.name}\n`);
      }
    }
    return 0;
  }

  if (selection.selected.length === 0) {
    process.stderr.write('No targets selected. Adjust --filter/--exclude or pass --all/--project/--file/--discover.\n');
    return 1;
  }

  if (!args.json || args.verbose) {
    process.stderr.write(`Running ${selection.selected.length} target(s)...\n`);
  }

  // Build the CLI once in the parent, then point every child at the prebuilt
  // binary and skip their own build step. Children inherit this environment via
  // spawn, so the cargo freshness check runs once instead of per target.
  const surgeBin = resolveSurgeBin();
  process.env.SURGE_TS_BIN = surgeBin;
  process.env.SURGE_TS_SKIP_BUILD = '1';

  const started = Date.now();
  const outcomes = await runSweep(selection.selected, args);
  const elapsedMs = Date.now() - started;
  const results = outcomes.map((outcome) => outcome.result);
  const summary = buildSummary(results, selection.skipped, elapsedMs);

  if (args.json) {
    const payload = {
      selected: selection.selected.map((target) => target.name),
      skipped: selection.skipped.map((target) => target.name),
      results,
      summary,
    };
    process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`);
    if (args.verbose) {
      for (const outcome of outcomes) {
        if (outcome.comparison) {
          process.stderr.write(`\n# ${outcome.result.preset}\n${renderComparisonText(outcome.comparison)}`);
        }
      }
    }
    return summary.exitCode;
  }

  const lines: string[] = [];
  for (const outcome of outcomes) {
    lines.push(formatPresetLine(outcome.result));
    if (outcome.result.error) {
      lines.push(indent(outcome.result.error));
    } else if (args.verbose && outcome.comparison) {
      lines.push(indent(renderComparisonText(outcome.comparison).trimEnd()));
    } else if (!outcome.result.passed && outcome.comparison) {
      lines.push(indent(formatFailureDetail(outcome.comparison)));
    }
  }
  lines.push('');
  lines.push(formatSummaryText(summary));
  process.stdout.write(`${lines.join('\n')}\n`);

  return summary.exitCode;
}

function usageText(): string {
  return [
    'Oracle sweep runner',
    '',
    'Run the TypeScript oracle comparison across many targets: registered presets,',
    'explicit tsconfig/source paths, or whole directories of projects.',
    '',
    'Usage:',
    '  pnpm run oracle:sweep -- --all',
    '  pnpm run oracle:sweep -- --filter <substring>',
    '  pnpm run oracle:sweep -- --project <tsconfig|dir|preset>',
    '  pnpm run oracle:sweep -- --file <source.ts>',
    '  pnpm run oracle:sweep -- --discover <dir>',
    '  pnpm run oracle:sweep -- --list',
    '',
    'Examples:',
    '  pnpm run oracle:sweep -- --list --all',
    '  pnpm run oracle:sweep -- --filter node-protocol --maxDiagnostics 200',
    '  pnpm run oracle:sweep -- --all --exclude diagnostics-pack',
    '  pnpm run oracle:sweep -- --project .local-projects/app/tsconfig.json',
    '  pnpm run oracle:sweep -- --discover tests/compat-projects --jobs 4',
    '',
    'Run with --help for the full flag list.',
    '',
  ].join('\n');
}

function printHelpAndExit(): never {
  process.stdout.write(
    [
      'Usage:',
      '  pnpm run oracle:sweep -- --all',
      '  pnpm run oracle:sweep -- --filter <substring>',
      '  pnpm run oracle:sweep -- --project <tsconfig|dir|preset>',
      '  pnpm run oracle:sweep -- --discover <dir>',
      '',
      'Targets:',
      '  --all                 Select every registered oracle preset.',
      '  --filter <substring>  Keep presets/discovered targets whose name includes the substring (repeatable).',
      '  --exclude <substring> Skip targets whose name includes the substring (repeatable).',
      '  --project <path|preset>  Add a tsconfig.json, project directory, or preset name (repeatable).',
      '  --file <path>         Add a single TypeScript source file (repeatable).',
      '  --discover <dir>      Recursively add every tsconfig.json under a directory (repeatable).',
      '  --list                List the selected target names and exit.',
      '',
      'Comparison:',
      '  --maxDiagnostics <n>  Limit diagnostics on both sides, passed to oracle compare.',
      '  --jobs <n>            Run targets concurrently (default: min(cpus, 4)).',
      '  --strictMessages      Fail targets that only differ in message text.',
      '  --strictSpans         Fail targets that only differ in line/column/span.',
      '  --json                Emit a machine-readable summary object.',
      '  --verbose             Include full per-target oracle output.',
      '  --help                Show this message.',
      '',
      'Default gate: a target fails on diagnostic code-count or file/code/line mismatch.',
      'Message-text and span/column differences are reported but do not fail by default.',
      '',
      'Examples:',
      '  pnpm run oracle:sweep -- --list --all',
      '  pnpm run oracle:sweep -- --filter node-protocol --maxDiagnostics 200',
      '  pnpm run oracle:sweep -- --project tests/compat-projects/generics-basic/tsconfig.json',
      '  pnpm run oracle:sweep -- --discover tests/compat-projects --exclude diagnostics-pack',
      '',
    ].join('\n'),
  );
  process.exit(0);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main()
    .then((code) => {
      process.exitCode = code;
    })
    .catch((error) => {
      const message = error instanceof Error ? error.message : String(error);
      process.stderr.write(`${message}\n`);
      process.exitCode = 1;
    });
}
