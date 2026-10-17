#!/usr/bin/env tsx

// Diagnostic-accuracy benchmark: scores surge-ts, bolt-ts and ts-rust against tsgo
// (TypeScript 7.0, the oracle baseline). A reported diagnostic counts only when
// it equals a tsgo diagnostic in file, line, column, code, category, span and
// full message (diagnostics.ts). See README.md in this directory for what the
// numbers do and do not mean.

import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

import {
  normalizeDiagnosticFileName,
  resolveProjectPresetOrPath,
} from '../oracle/compare-tsc.js';
import { discoverProjectTargets } from '../oracle/sweep-presets.js';
import {
  attachApiSpans,
  attachByteSpans,
  mergeSurgeOutputs,
  mismatchMeanings,
  parseBoltOutput,
  parseSurgeJson,
  parseTscStyleOutput,
  rangeFromBytes,
  realPath,
  recoverBoltSpan,
  scoreStrict,
  SourceText,
  type ApiDiagnostic,
  type BoltDiagnostic,
  type FullDiagnostic,
  type MessageMode,
  type MismatchExample,
  type MismatchField,
  type StrictScore,
} from './diagnostics.js';
import {
  escapeHtml,
  metaGridHtml,
  renderPanelsSvg,
  renderReportPage,
  type DriftStyle,
  type Panel,
  type PanelGroup,
  type PanelRow,
} from './report.js';
import {
  boltTestSuite,
  listUpstreamCases,
  materializeCase,
  selectHeldOutCases,
  surgeTestSuite,
} from './upstream-cases.js';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const workspaceRoot = path.resolve(scriptDir, '../..');
const tsgoBinPath = path.join(workspaceRoot, 'node_modules', 'typescript', 'bin', 'tsc');
const tsgoApiScript = path.join(scriptDir, 'tsgo-api-diagnostics.mjs');
const baselineCacheDir = path.join(workspaceRoot, '.bench', 'checkers-cache', 'tsgo');
// bolt-ts emits message text only, so codes are recovered from the TS 6 JS
// compiler's diagnostic table (TS 7 embeds its table in the Go binary).
const tsDiagnosticTablePath = path.join(workspaceRoot, 'node_modules', 'typescript-6', 'lib', 'typescript.js');
const defaultBoltBin = path.resolve(workspaceRoot, '..', 'bolt-ts', 'target', 'release', 'bolt_ts_compiler');
const defaultSurgeBin = path.join(workspaceRoot, 'target', 'release', 'surge');
// ts-rust (github.com/pingdotgg/ts-rust) ships as the `tsc-rs` devDependency;
// its native tsc is run directly, skipping the Node launcher in front of it.
function defaultTsRustBin(): string {
  try {
    const require = createRequire(path.join(workspaceRoot, 'node_modules', 'tsc-rs', 'package.json'));
    const platformPackage = require.resolve(`@tsc-rs/${process.platform}-${process.arch}/package.json`);
    return path.join(path.dirname(platformPackage), 'lib', process.platform === 'win32' ? 'tsc.exe' : 'tsc');
  } catch {
    return path.join(workspaceRoot, 'node_modules', '@tsc-rs', `${process.platform}-${process.arch}`, 'lib', 'tsc');
  }
}
// TypeScript's test cases at the submodule commit typescript-go pins, plus
// typescript-go's own cases. See README.md for the checkout commands.
const upstreamCaseRoots = [
  path.join(workspaceRoot, '.local-projects', 'typescript-cases', 'tests', 'cases'),
  path.join(workspaceRoot, '.local-projects', 'typescript-go-cases', 'testdata', 'tests', 'cases'),
];

export const corpusTargets: Record<string, string> = {
  ky: '.local-projects/ky/tsconfig.json',
  ofetch: '.local-projects/ofetch/tsconfig.json',
  zod: '.local-projects/zod/tsconfig.json',
  zustand: '.local-projects/zustand/tsconfig.json',
  trpc: '.local-projects/trpc/tsconfig.json',
  'tanstack-query': '.local-projects/tanstack-query/tsconfig.surge.json',
  'ts-pattern': '.local-projects/ts-pattern/tsconfig.surge.json',
  'drizzle-orm': '.local-projects/drizzle-orm/drizzle-orm/tsconfig.surge.json',
};

export type Checker = 'surge-ts' | 'bolt-ts' | 'ts-rust';
const allCheckers: Checker[] = ['surge-ts', 'bolt-ts', 'ts-rust'];

// Reports show the checkers a run scored, so a `--fromJson` rerender of an
// older two-checker report keeps its columns.
function checkersIn(results: TargetResult[]): Checker[] {
  return allCheckers.filter((checker) => results.some((result) => result.scores[checker]));
}

export type Tier = 'upstream' | 'fixture' | 'corpus';

export type Target = { name: string; tier: Tier; tsconfig: string };

// `no-input`: the checker exited cleanly but loaded no project source, so its
// silence says nothing about accuracy.
// `invalid-config`: tsgo rejected the upstream case's options (TS5xxx), so the
// case measures config validation rather than checking and is not scored.
// `memory-limit`: killed by the harness's resident-memory cap; not scored.
export type RunStatus = 'ok' | 'crash' | 'timeout' | 'memory-limit' | 'no-input' | 'invalid-config';

export type CheckerScore = StrictScore & {
  status: RunStatus;
  reason?: string;
  ms: number;
  reported: number;
  // Emitted in a file tsgo does not consider part of the program (a stray
  // emitted `.js` beside its source, a lib file); neither FP nor exact.
  outOfProgram: number;
  // bolt-ts messages no TS diagnostic template matched; excluded from FP.
  unmapped: number;
  unmappedMessages: string[];
  // What the bolt-ts config translation could not carry over.
  configNotes?: string[];
  // Scored diagnostics, and how many of them showed a category and a span.
  observed?: { diagnostics: number; category: number; span: number };
  spanNotes?: Record<string, number>;
};

export type TargetResult = {
  name: string;
  tier: Tier;
  tsconfig: string;
  baselineStatus: RunStatus;
  baselineReason?: string;
  baseline: number;
  // Of tsgo's diagnostics, those with a file, and how many of them the API gave
  // a span and related information.
  baselineSpans?: { located: number; withSpan: number; withRelated: number; api: 'ok' | 'failed' };
  scores: Partial<Record<Checker, CheckerScore>>;
};

type ParsedArgs = {
  upstream: boolean;
  upstreamLimit: number;
  fixtures: boolean;
  corpora: boolean;
  projects: string[];
  filters: string[];
  jobs: number;
  timeoutMs: number;
  corpusTimeoutMs: number;
  maxMemoryMb: number;
  corpusMaxMemoryMb: number;
  out: string;
  boltBin: string;
  surgeBin: string;
  tsRustBin: string;
  baselineCache: boolean;
  checkers: Checker[];
  fromJson?: string;
};

type ProcessOutput = {
  status: number | null;
  timedOut: boolean;
  memoryExceeded: boolean;
  stdout: string;
  stderr: string;
  ms: number;
};

// macOS enforces no address-space rlimit, and a runaway checker has taken this
// 16 GB machine down well inside the wall-clock timeout, so every spawned tool
// is polled for memory and its process group killed past the cap.
const memoryPollMs = 100;

function residentBytes(pid: number): number {
  const result = spawnSync('ps', ['-o', 'rss=', '-g', String(pid)], { encoding: 'utf8' });
  return result.stdout
    .split('\n')
    .map((line) => Number(line.trim()))
    .filter((kb) => Number.isFinite(kb))
    .reduce((total, kb) => total + kb * 1024, 0);
}

// Resident size is the wrong measure on macOS: under pressure the kernel
// compresses and swaps a process's pages out of its resident set, so `ps rss`
// stays under the cap while the process keeps growing. Five bolt-ts runs
// reached 12–23 GB each that way and panicked the machine. There a process
// group is measured by physical footprint (`proc_pid_rusage`, what jetsam
// charges), through one sidecar the whole run shares.
const footprintSidecar = `
import ctypes, sys
libproc = ctypes.CDLL('/usr/lib/libproc.dylib')
class Usage(ctypes.Structure):
    _fields_ = [('uuid', ctypes.c_uint8 * 16)] + [(name, ctypes.c_uint64) for name in (
        'user_time', 'system_time', 'pkg_idle_wkups', 'interrupt_wkups', 'pageins',
        'wired_size', 'resident_size', 'phys_footprint', 'proc_start_abstime', 'proc_exit_abstime',
        'child_user_time', 'child_system_time', 'child_pkg_idle_wkups', 'child_interrupt_wkups',
        'child_pageins', 'child_elapsed_abstime', 'diskio_bytesread', 'diskio_byteswritten')]
members = (ctypes.c_int * 4096)()
usage = Usage()
for line in sys.stdin:
    totals = []
    for group in line.split():
        count = libproc.proc_listpgrppids(int(group), members, ctypes.sizeof(members))
        total = 0
        for pid in members[:max(count, 0)]:
            if pid > 0 and libproc.proc_pid_rusage(pid, 2, ctypes.byref(usage)) == 0:
                total += usage.phys_footprint
        totals.append('%s:%d' % (group, total))
    print(' '.join(totals), flush=True)
`;

type MemoryWatch = { group: number; maxBytes: number; exceeded: () => void };

const memoryWatches = new Set<MemoryWatch>();
let memoryTimer: NodeJS.Timeout | undefined;
let footprintProbe: ChildProcess | undefined;
let footprintProbeUnavailable = process.platform !== 'darwin';
let footprintPending = false;

function watchMemory(watch: MemoryWatch): () => void {
  memoryWatches.add(watch);
  memoryTimer ??= setInterval(pollMemory, memoryPollMs);
  return () => {
    memoryWatches.delete(watch);
    if (memoryWatches.size === 0 && memoryTimer) {
      clearInterval(memoryTimer);
      memoryTimer = undefined;
    }
  };
}

function pollMemory() {
  const probe = footprintProbeUnavailable ? undefined : startFootprintProbe();
  if (!probe) {
    for (const watch of [...memoryWatches]) {
      if (residentBytes(watch.group) > watch.maxBytes) watch.exceeded();
    }
    return;
  }
  if (footprintPending || memoryWatches.size === 0) return;
  footprintPending = true;
  probe.stdin?.write(`${[...memoryWatches].map((watch) => watch.group).join(' ')}\n`);
}

function startFootprintProbe(): ChildProcess | undefined {
  if (footprintProbe) return footprintProbe;
  const probe = spawn('python3', ['-c', footprintSidecar], { stdio: ['pipe', 'pipe', 'ignore'] });
  let buffered = '';
  const retire = () => {
    if (footprintProbe === probe) footprintProbe = undefined;
    footprintPending = false;
  };
  probe.on('error', () => {
    footprintProbeUnavailable = true;
    retire();
  });
  probe.on('exit', retire);
  probe.stdout?.on('data', (chunk: Buffer) => {
    buffered += chunk.toString('utf8');
    let newline;
    while ((newline = buffered.indexOf('\n')) >= 0) {
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      footprintPending = false;
      for (const entry of line.split(' ')) {
        const [group, bytes] = entry.split(':').map(Number);
        for (const watch of [...memoryWatches]) {
          if (watch.group === group && bytes > watch.maxBytes) watch.exceeded();
        }
      }
    }
  });
  // The sidecar must never keep the harness alive; the poll timer does while
  // anything is watched.
  probe.unref();
  (probe.stdin as unknown as { unref?: () => void } | null)?.unref?.();
  (probe.stdout as unknown as { unref?: () => void } | null)?.unref?.();
  footprintProbe = probe;
  return probe;
}

export function runProcess(
  command: string,
  args: string[],
  options: { cwd: string; timeoutMs: number; maxMemoryBytes: number; env?: NodeJS.ProcessEnv },
): Promise<ProcessOutput> {
  return new Promise((resolve) => {
    const started = performance.now();
    // Its own process group, so a kill also reaches anything it spawned.
    const child = spawn(command, args, { cwd: options.cwd, env: options.env ?? process.env, detached: true });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let timedOut = false;
    let memoryExceeded = false;
    const killGroup = () => {
      try {
        if (child.pid !== undefined) process.kill(-child.pid, 'SIGKILL');
      } catch {
        child.kill('SIGKILL');
      }
    };
    const timer = setTimeout(() => {
      timedOut = true;
      killGroup();
    }, options.timeoutMs);
    const unwatch =
      child.pid === undefined
        ? () => {}
        : watchMemory({
            group: child.pid,
            maxBytes: options.maxMemoryBytes,
            exceeded: () => {
              memoryExceeded = true;
              killGroup();
            },
          });
    const stop = () => {
      clearTimeout(timer);
      unwatch();
    };
    child.stdout.on('data', (chunk: Buffer) => stdout.push(chunk));
    child.stderr.on('data', (chunk: Buffer) => stderr.push(chunk));
    child.on('error', (error) => {
      stop();
      resolve({ status: null, timedOut, memoryExceeded, stdout: '', stderr: String(error), ms: performance.now() - started });
    });
    child.on('close', (status) => {
      stop();
      resolve({
        status,
        timedOut,
        memoryExceeded,
        stdout: Buffer.concat(stdout).toString('utf8'),
        stderr: Buffer.concat(stderr).toString('utf8'),
        ms: performance.now() - started,
      });
    });
  });
}

// bolt-ts messages that shorten a tsc template without changing which check
// fired. Only one-to-one rewordings belong here; anything ambiguous stays
// unmapped.
const boltRewordings: Array<[string, string]> = [
  ['TS2741', "Property '{0}' is missing."],
  ['TS2314', "Generic type '{0}' requires {1} type argument."],
  ['TS2314', "Generic type '{0}' requires {1} type arguments."],
  ['TS2676', 'Get and set accessors in a class must both be abstract or non-abstract.'],
];

type MessageTemplate = { code: string; prefix: string; pattern: RegExp; literalLength: number };

let cachedTemplates: MessageTemplate[] | null = null;

export function loadMessageTemplates(sourcePath = tsDiagnosticTablePath): MessageTemplate[] {
  if (cachedTemplates) {
    return cachedTemplates;
  }
  const source = readFileSync(sourcePath, 'utf8');
  const templates: MessageTemplate[] = [];
  const diag = /diag\(\s*(\d+),\s*(\d+)[^,]*,\s*"[^"]*",\s*"((?:[^"\\]|\\.)*)"/g;
  for (const match of source.matchAll(diag)) {
    // Category 1 is Error; warnings/suggestions/messages never reach tsc's
    // `error TSxxxx` output, so admitting them only creates false mappings.
    if (match[2] !== '1') {
      continue;
    }
    const text = JSON.parse(`"${match[3]}"`) as string;
    templates.push(compileTemplate(`TS${match[1]}`, text));
  }
  for (const [code, text] of boltRewordings) {
    templates.push(compileTemplate(code, text));
  }
  cachedTemplates = templates;
  return templates;
}

// Whitespace is collapsed on both sides: several tsc templates contain double
// spaces, and miette's 80-column wrapping cannot preserve them.
export function compileTemplate(code: string, text: string): MessageTemplate {
  const parts = collapseWhitespace(text).split(/\{\d+\}/);
  const pattern = new RegExp(`^${parts.map(escapeRegExp).join('([\\s\\S]*?)')}$`);
  return { code, prefix: parts[0], pattern, literalLength: parts.join('').length };
}

function collapseWhitespace(value: string): string {
  return value.replace(/\s+/g, ' ').trim();
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// The most literal template wins, so `Cannot find name '{0}'. Did you mean
// '{1}'?` beats `Cannot find name '{0}'.` when both could match.
export function codeForMessage(rawMessage: string, templates: MessageTemplate[]): string | null {
  const message = collapseWhitespace(rawMessage);
  let best: MessageTemplate | null = null;
  for (const template of templates) {
    if (!message.startsWith(template.prefix)) {
      continue;
    }
    if (best && template.literalLength <= best.literalLength) {
      continue;
    }
    if (template.pattern.test(message)) {
      best = template;
    }
  }
  return best?.code ?? null;
}

function emptyScore(status: RunStatus, ms: number, reason?: string): CheckerScore {
  return {
    status,
    reason,
    ms,
    reported: 0,
    exact: 0,
    fp: 0,
    fn: 0,
    mismatched: 0,
    fields: {},
    spanless: 0,
    fpKeys: [],
    fnKeys: [],
    examples: [],
    outOfProgram: 0,
    unmapped: 0,
    unmappedMessages: [],
  };
}

function failureReason(output: ProcessOutput): string {
  const text = `${output.stderr}\n${output.stdout}`;
  const panic = /panicked at ([^\n]+)\n([^\n]*)/.exec(text);
  if (panic) {
    return `${panic[1].replace(/:\d+:\d+:$/, '')} ${panic[2]}`.trim().slice(0, 240);
  }
  return (text.trim().split('\n').pop() ?? '').slice(0, 240);
}

function limitStatus(output: ProcessOutput): RunStatus | null {
  if (output.memoryExceeded) return 'memory-limit';
  if (output.timedOut) return 'timeout';
  return null;
}

type CheckerRun = { output: ProcessOutput; diagnostics: unknown[] | null; status: RunStatus; notes?: string[] };

// Two runs: the JSON report carries byte spans, the tsc-style text the
// category; neither carries both.
async function runSurge(target: Target, args: ParsedArgs, limits: RunLimits): Promise<CheckerRun> {
  const projectDir = path.dirname(target.tsconfig);
  const run = (format: string[]) =>
    runProcess(args.surgeBin, ['--project', target.tsconfig, ...format, '--maxDiagnostics', '1000000'], { cwd: workspaceRoot, ...limits });
  const output = await run(['--format', 'json']);
  const limited = limitStatus(output);
  if (limited) {
    return { output, diagnostics: null, status: limited };
  }
  let json: FullDiagnostic[];
  try {
    json = parseSurgeJson(output.stdout, projectDir);
  } catch {
    return { output, diagnostics: null, status: 'crash' };
  }
  const text = await run(['--diagnosticStyle', 'tsc', '--pretty', 'false']);
  const textDiagnostics = limitStatus(text) ? null : parseTscStyleOutput(text.stdout, projectDir, workspaceRoot);
  return { output, diagnostics: mergeSurgeOutputs(json, textDiagnostics), status: 'ok' };
}

async function runBolt(target: Target, args: ParsedArgs, limits: RunLimits): Promise<CheckerRun> {
  const projectDir = path.dirname(target.tsconfig);
  const translated = translateConfigForBolt(target.tsconfig);
  const run = (config: BoltConfig) =>
    runProcess(args.boltBin, [writeBoltConfig(target, config)], {
      cwd: projectDir,
      ...limits,
      env: { ...process.env, NO_COLOR: '1' },
    });
  const output = await run(translated.config);
  const limited = limitStatus(output);
  if (limited) {
    return { output, diagnostics: null, status: limited, notes: translated.notes };
  }
  if (output.status !== 0) {
    return { output, diagnostics: null, status: 'crash', notes: translated.notes };
  }
  // bolt-ts exits 0 when its include matches nothing. Its `Files:` count
  // includes the bundled lib files, so compare against the same options with
  // an empty `include` to tell "no errors" from "checked nothing".
  const libOnly = await run({ ...translated.config, include: [] });
  const files = filesCount(output.stdout);
  const libFiles = filesCount(libOnly.stdout);
  if (files !== null && libFiles !== null && files <= libFiles) {
    return {
      output: { ...output, stderr: `loaded ${files} files, all bundled libs` },
      diagnostics: null,
      status: 'no-input',
      notes: translated.notes,
    };
  }
  return { output, diagnostics: parseBoltOutput(output.stdout), status: 'ok', notes: translated.notes };
}

// ts-rust takes tsc's command line, so it is read exactly like the baseline:
// `--pretty false` text for the diagnostic set, its port of tsgo's API for spans.
async function runTsRust(target: Target, args: ParsedArgs, limits: RunLimits, readSource: SourceReader): Promise<CheckerRun> {
  const projectDir = path.dirname(target.tsconfig);
  const output = await runProcess(args.tsRustBin, ['--noEmit', '--pretty', 'false', '--project', target.tsconfig], {
    cwd: workspaceRoot,
    ...limits,
  });
  const limited = limitStatus(output);
  if (limited) {
    return { output, diagnostics: null, status: limited };
  }
  // tsc exits 1 or 2 when it reports diagnostics; anything else (a panic is
  // 101, a signal is null) did not finish the check.
  if (output.status === null || output.status > 2) {
    return { output, diagnostics: null, status: 'crash' };
  }
  const diagnostics = parseTscStyleOutput(`${output.stdout}${output.stderr}`, projectDir, workspaceRoot);
  const api = await runProcess(process.execPath, [tsgoApiScript, target.tsconfig, 'tsc-rs', args.tsRustBin], { cwd: workspaceRoot, ...limits });
  const apiDiagnostics = parseApiOutput(api);
  if (apiDiagnostics) attachApiSpans(diagnostics, apiDiagnostics, projectDir, readSource);
  return { output, diagnostics, status: 'ok' };
}

type BoltConfig = { compilerOptions: Record<string, unknown>; include: string[] };

type TypeScriptApi = {
  sys: unknown;
  getParsedCommandLineOfConfigFile(
    configFileName: string,
    optionsToExtend: object,
    host: unknown,
  ): { options: Record<string, unknown>; fileNames: string[] } | undefined;
  getEmitScriptTarget(options: object): number;
  getEmitModuleKind(options: object): number;
  getEmitModuleResolutionKind(options: object): number;
  getStrictOptionValue(options: object, flag: string): boolean;
  ScriptTarget: Record<string, number>;
  ModuleKind: Record<string, number>;
  ModuleResolutionKind: Record<string, number>;
  JsxEmit: Record<string, number>;
};

let typescriptApi: TypeScriptApi | null = null;

function loadTypeScript(): TypeScriptApi {
  typescriptApi ??= createRequire(import.meta.url)(tsDiagnosticTablePath) as TypeScriptApi;
  return typescriptApi;
}

// Variant names bolt-ts's config enums deserialize (crates/config/src/raw).
const boltTargets = ['ES5', 'ES2015', 'ES2016', 'ES2017', 'ES2018', 'ES2019', 'ES2020', 'ES2021', 'ES2022', 'ES2023', 'ES2024', 'ES2025', 'ESNext'];
const boltModules = ['None', 'CommonJS', 'AMD', 'UMD', 'System', 'ES2015', 'ES2020', 'ES2022', 'ESNext', 'Node16', 'Node18', 'Node20', 'NodeNext', 'Preserve'];
const boltModuleResolutions = ['Classic', 'Node10', 'Node16', 'NodeNext', 'Bundler'];
const boltJsx = ['Preserve', 'React', 'ReactNative', 'ReactJSX', 'ReactJSXDev'];
const boltLibs = new Set([
  'es5', 'es2015', 'es6', 'es2016', 'es7', 'es2017', 'es2018', 'es2019', 'es2020', 'es2021', 'es2022', 'es2023', 'esnext',
  'dom', 'webworker', 'scripthost', 'dom.iterable',
  'es2015.core', 'es2015.collection', 'es2015.generator', 'es2015.iterable', 'es2015.promise', 'es2015.proxy',
  'es2015.reflect', 'es2015.symbol', 'es2015.symbol.wellknown', 'es2016.array.include', 'es2017.object', 'es2017.intl',
  'es2017.sharedmemory', 'es2017.string', 'es2017.typedarrays', 'es2018.intl', 'es2018.promise', 'es2018.regexp',
  'es2019.array', 'es2019.object', 'es2019.string', 'es2019.symbol', 'es2020.string', 'es2020.symbol.wellknown',
  'es2021.promise', 'es2021.weakref', 'esnext.asynciterable', 'esnext.array', 'esnext.intl', 'esnext.symbol',
]);
const boltBooleanOptions = [
  'declaration', 'strict', 'strictNullChecks', 'strictFunctionTypes', 'strictBindCallApply', 'strictPropertyInitialization',
  'noImplicitAny', 'noImplicitThis', 'noImplicitReturns', 'noUncheckedIndexedAccess', 'noStrictGenericChecks',
  'noFallthroughCasesInSwitch', 'noErrorTruncation', 'noUnusedLocals', 'noUnusedParameters', 'alwaysStrict',
  'allowUnusedLabels', 'allowUnreachableCode', 'esModuleInterop', 'exactOptionalPropertyTypes', 'preserveSymlinks',
  'useDefineForClassFields', 'useUnknownInCatchVariables', 'resolveJsonModule', 'resolvePackageJsonExports',
  'resolvePackageJsonImports', 'removeComments', 'checkJs',
];
const strictFamily = [
  'noImplicitAny', 'noImplicitThis', 'strictNullChecks', 'strictFunctionTypes', 'strictBindCallApply',
  'strictPropertyInitialization', 'alwaysStrict', 'useUnknownInCatchVariables',
];
// Options that change what tsgo reports but bolt-ts has no field for.
const boltUnsupportedOptions = ['paths', 'baseUrl', 'types', 'typeRoots', 'rootDirs', 'customConditions', 'allowJs', 'skipLibCheck', 'noLib', 'verbatimModuleSyntax', 'isolatedModules'];

function enumName(values: Record<string, number>, value: number, accepted: string[]): string | undefined {
  return accepted.find((name) => values[name] === value);
}

// bolt-ts reads none of `extends`/`files`, cannot expand a bare directory in
// `include`, and rejects option spellings tsc accepts (`nodenext`, `DOM`).
// TypeScript resolves the config instead and bolt-ts gets the result: the
// program's root files listed explicitly, and each effective option (tsgo's
// defaults included) spelled the way bolt-ts deserializes it.
export function translateConfigForBolt(tsconfig: string): { config: BoltConfig; notes: string[] } {
  const ts = loadTypeScript();
  const notes: string[] = [];
  const parsed = ts.getParsedCommandLineOfConfigFile(tsconfig, {}, {
    ...(ts.sys as object),
    onUnRecoverableConfigFileDiagnostic: () => {},
  });
  if (!parsed) {
    return { config: { compilerOptions: {}, include: [] }, notes: ['config unreadable by TypeScript'] };
  }
  const options = parsed.options;
  const compilerOptions: Record<string, unknown> = {};

  const target = ts.getEmitScriptTarget(options);
  const targetName = enumName(ts.ScriptTarget, target, boltTargets);
  if (targetName) compilerOptions.target = targetName;
  else notes.push(`target ${target} unsupported`);
  const moduleName = enumName(ts.ModuleKind, ts.getEmitModuleKind(options), boltModules);
  if (moduleName) compilerOptions.module = moduleName;
  const resolutionName = enumName(ts.ModuleResolutionKind, ts.getEmitModuleResolutionKind(options), boltModuleResolutions);
  if (resolutionName) compilerOptions.moduleResolution = resolutionName;
  if (typeof options.jsx === 'number') {
    const jsxName = enumName(ts.JsxEmit, options.jsx, boltJsx);
    if (jsxName) compilerOptions.jsx = jsxName;
  }

  if (Array.isArray(options.lib)) {
    const libs: string[] = [];
    for (const file of options.lib as string[]) {
      const lib = file.replace(/^lib\./, '').replace(/\.d\.ts$/, '').toLowerCase();
      if (boltLibs.has(lib)) libs.push(lib);
      else notes.push(`lib ${lib} dropped`);
    }
    compilerOptions.lib = libs;
  }

  for (const flag of boltBooleanOptions) {
    if (typeof options[flag] === 'boolean') compilerOptions[flag] = options[flag];
  }
  for (const flag of strictFamily) {
    compilerOptions[flag] = ts.getStrictOptionValue(options, flag);
  }
  for (const option of boltUnsupportedOptions) {
    if (options[option] !== undefined) notes.push(`${option} not supported`);
  }

  return { config: { compilerOptions, include: parsed.fileNames }, notes };
}

function writeBoltConfig(target: Target, config: BoltConfig): string {
  const variant = config.include.length === 0 ? 'lib-only' : 'config';
  const dir = path.join(os.tmpdir(), 'surge-compare-checkers', target.name.replace(/[^\w.-]+/g, '_'), variant);
  mkdirSync(dir, { recursive: true });
  // bolt-ts reads a config only when the path's last component is exactly
  // `tsconfig.json`; any other name becomes an include glob.
  const file = path.join(dir, 'tsconfig.json');
  writeFileSync(file, JSON.stringify(config, null, 2));
  return file;
}

function filesCount(stdout: string): number | null {
  const match = /^Files: (\d+)$/m.exec(stdout);
  return match ? Number(match[1]) : null;
}

type RunLimits = { timeoutMs: number; maxMemoryBytes: number };

type BaselineRuns = { tsgo: ProcessOutput; listed: ProcessOutput; api?: ProcessOutput };

// Only materialized upstream cases are cached: their directory is the whole
// input, whereas fixtures and corpora read files outside it (shared sources,
// node_modules) that a directory hash would miss. The directory's own path is
// keyed too, because tsgo's output names files by path.
export function baselineCacheKey(projectDir: string, tsgoVersion: string): string {
  const hash = createHash('sha256').update(`tsgo ${tsgoVersion}\0${path.resolve(projectDir)}\0`);
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else hash.update(`${path.relative(projectDir, full)}\0`).update(readFileSync(full)).update('\0');
    }
  };
  walk(projectDir);
  return hash.digest('hex');
}

async function runBaseline(target: Target, args: ParsedArgs, limits: RunLimits): Promise<BaselineRuns> {
  const cacheFile = args.baselineCache && target.tier === 'upstream'
    ? path.join(baselineCacheDir, `${baselineCacheKey(path.dirname(target.tsconfig), readPackageVersion('typescript'))}.json`)
    : null;
  if (cacheFile && existsSync(cacheFile)) {
    try {
      const cached = JSON.parse(readFileSync(cacheFile, 'utf8')) as BaselineRuns;
      // Entries written before the API run was added lack it.
      if (cached.api) return cached;
    } catch {
      // A torn or stale entry is recomputed and overwritten below.
    }
  }
  const tsgo = await runProcess(
    process.execPath,
    [tsgoBinPath, '--noEmit', '--pretty', 'false', '--project', target.tsconfig],
    { cwd: workspaceRoot, ...limits },
  );
  if (limitStatus(tsgo)) return { tsgo, listed: tsgo };
  const listed = await runProcess(
    process.execPath,
    [tsgoBinPath, '--listFilesOnly', '--project', target.tsconfig],
    { cwd: workspaceRoot, ...limits },
  );
  const api = await runProcess(process.execPath, [tsgoApiScript, target.tsconfig], { cwd: workspaceRoot, ...limits });
  // Limit hits depend on --timeout/--maxMemory and machine load, not on the case.
  if (cacheFile && !limitStatus(listed) && !limitStatus(api)) {
    mkdirSync(baselineCacheDir, { recursive: true });
    const temp = `${cacheFile}.${process.pid}.tmp`;
    writeFileSync(temp, JSON.stringify({ tsgo, listed, api }));
    renameSync(temp, cacheFile);
  }
  return { tsgo, listed, api };
}

function parseApiOutput(api: ProcessOutput | undefined): ApiDiagnostic[] | null {
  if (!api || api.status !== 0) return null;
  try {
    return (JSON.parse(api.stdout) as { diagnostics: ApiDiagnostic[] }).diagnostics;
  } catch {
    return null;
  }
}

type SourceReader = (file: string) => SourceText | null;

function sourceReader(): SourceReader {
  const cache = new Map<string, SourceText | null>();
  return (file) => {
    if (!cache.has(file)) cache.set(file, SourceText.read(file));
    return cache.get(file) ?? null;
  };
}

async function evaluateTarget(target: Target, args: ParsedArgs): Promise<TargetResult> {
  const corpus = target.tier === 'corpus';
  const limits: RunLimits = {
    timeoutMs: corpus ? args.corpusTimeoutMs : args.timeoutMs,
    maxMemoryBytes: (corpus ? args.corpusMaxMemoryMb : args.maxMemoryMb) * 1024 * 1024,
  };
  const projectDir = path.dirname(target.tsconfig);
  const result: TargetResult = {
    name: target.name,
    tier: target.tier,
    tsconfig: path.relative(workspaceRoot, target.tsconfig),
    baselineStatus: 'ok',
    baseline: 0,
    scores: {},
  };

  const { tsgo, listed, api } = await runBaseline(target, args, limits);
  const baselineLimited = limitStatus(tsgo);
  if (baselineLimited) {
    result.baselineStatus = baselineLimited;
    return result;
  }
  const readSource = sourceReader();
  const baseline = parseTscStyleOutput(`${tsgo.stdout}${tsgo.stderr}`, projectDir, workspaceRoot);
  result.baseline = baseline.length;
  if (target.tier === 'upstream' && baseline.some((diagnostic) => /^TS5\d{3}$/.test(diagnostic.code))) {
    result.baselineStatus = 'invalid-config';
    return result;
  }
  const apiDiagnostics = parseApiOutput(api);
  if (apiDiagnostics) attachApiSpans(baseline, apiDiagnostics, projectDir, readSource);
  const located = baseline.filter((diagnostic) => diagnostic.fileName);
  result.baselineSpans = {
    located: located.length,
    withSpan: located.filter((diagnostic) => diagnostic.span).length,
    withRelated: located.filter((diagnostic) => diagnostic.related?.length).length,
    api: apiDiagnostics ? 'ok' : 'failed',
  };

  const programFiles =
    listed.status === 0
      ? new Set(
          listed.stdout
            .split(/\r?\n/)
            .filter(Boolean)
            .map((file) => normalizeDiagnosticFileName(projectDir, realPath(path.resolve(workspaceRoot, file)))),
        )
      : null;

  if (args.checkers.includes('surge-ts')) {
    const surge = await runSurge(target, args, limits);
    result.scores['surge-ts'] = scoreRun(surge, baseline, programFiles, 'exact', (diagnostics) => ({
      normalized: attachByteSpans(diagnostics as FullDiagnostic[], readSource),
      unmapped: [],
    }));
  }

  if (args.checkers.includes('bolt-ts')) {
    const bolt = await runBolt(target, args, limits);
    // miette rewraps messages at 80 columns, so whitespace runs are not observable.
    const boltScore = scoreRun(bolt, baseline, programFiles, 'whitespace', (diagnostics) =>
      mapBoltDiagnostics(diagnostics as BoltDiagnostic[], projectDir, readSource),
    );
    boltScore.configNotes = bolt.notes;
    result.scores['bolt-ts'] = boltScore;
  }

  if (args.checkers.includes('ts-rust')) {
    const tsRust = await runTsRust(target, args, limits, readSource);
    const tsRustScore = scoreRun(tsRust, baseline, programFiles, 'exact', (diagnostics) => ({
      normalized: diagnostics as FullDiagnostic[],
      unmapped: [],
    }));
    result.scores['ts-rust'] = tsRustScore;
  }
  return result;
}

function scoreRun(
  run: CheckerRun,
  baseline: FullDiagnostic[],
  programFiles: Set<string> | null,
  mode: MessageMode,
  normalize: (diagnostics: unknown[]) => { normalized: FullDiagnostic[]; unmapped: FullDiagnostic[] },
): CheckerScore {
  if (!run.diagnostics) {
    const reason =
      run.status === 'timeout' || run.status === 'memory-limit'
        ? undefined
        : run.status === 'no-input'
          ? run.output.stderr
          : failureReason(run.output);
    return emptyScore(run.status, run.output.ms, reason);
  }
  const { normalized, unmapped } = normalize(run.diagnostics);
  const inProgram = (diagnostic: FullDiagnostic) =>
    !programFiles || !diagnostic.fileName || programFiles.has(diagnostic.fileName);
  const scored = normalized.filter(inProgram);
  const scoredUnmapped = unmapped.filter(inProgram);
  const spanNotes: Record<string, number> = {};
  for (const diagnostic of scored) {
    if (diagnostic.spanNote) spanNotes[diagnostic.spanNote] = (spanNotes[diagnostic.spanNote] ?? 0) + 1;
  }
  return {
    ...emptyScore('ok', run.output.ms),
    ...scoreStrict(baseline, scored, mode),
    reported: normalized.length + unmapped.length,
    outOfProgram: normalized.length - scored.length + unmapped.length - scoredUnmapped.length,
    unmapped: scoredUnmapped.length,
    unmappedMessages: scoredUnmapped.map((diagnostic) => diagnostic.message ?? ''),
    observed: {
      diagnostics: scored.length,
      category: scored.filter((diagnostic) => diagnostic.category).length,
      span: scored.filter((diagnostic) => diagnostic.span).length,
    },
    spanNotes,
  };
}

function mapBoltDiagnostics(
  diagnostics: BoltDiagnostic[],
  projectDir: string,
  readSource: SourceReader,
): { normalized: FullDiagnostic[]; unmapped: FullDiagnostic[] } {
  const templates = loadMessageTemplates();
  const normalized: FullDiagnostic[] = [];
  const unmapped: FullDiagnostic[] = [];
  for (const diagnostic of diagnostics) {
    const code = codeForMessage(diagnostic.message, templates);
    const absolute = diagnostic.fileName ? realPath(path.resolve(projectDir, diagnostic.fileName)) : '';
    const mapped: FullDiagnostic = {
      source: 'typescript',
      code: code ?? '',
      fileName: absolute ? normalizeDiagnosticFileName(projectDir, absolute) : '',
      path: absolute,
      line: diagnostic.line,
      column: diagnostic.column,
      message: diagnostic.message,
      category: diagnostic.category,
    };
    const source = absolute ? readSource(absolute) : null;
    if (source) {
      const recovered = recoverBoltSpan(diagnostic, source);
      const span = 'unobservable' in recovered ? null : rangeFromBytes(source, recovered.start, recovered.end);
      if (span) mapped.span = span;
      else mapped.spanNote = 'unobservable' in recovered ? recovered.unobservable : 'byte offset not on a character boundary';
    } else if (absolute) {
      mapped.spanNote = 'file unreadable';
    }
    (code ? normalized : unmapped).push(mapped);
  }
  return { normalized, unmapped };
}

async function runPool<T, R>(items: T[], jobs: number, worker: (item: T, index: number) => Promise<R>): Promise<R[]> {
  const results: R[] = new Array(items.length);
  let next = 0;
  const lanes = Array.from({ length: Math.max(1, Math.min(jobs, items.length)) }, async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await worker(items[index], index);
    }
  });
  await Promise.all(lanes);
  return results;
}

export type Aggregate = {
  targets: number;
  completed: number;
  crashed: number;
  timedOut: number;
  memoryLimited: number;
  noInput: number;
  exactTargets: number;
  exact: number;
  fp: number;
  fn: number;
  mismatched: number;
  spanless: number;
  fields: Partial<Record<MismatchField, number>>;
  outOfProgram: number;
  unmapped: number;
  precision: number | null;
  recall: number | null;
  f1: number | null;
  totalMs: number;
};

// Accuracy is summed only over targets every checker completed, so a crash
// cannot make a checker look better by dropping its hardest targets.
function commonCompleted(results: TargetResult[]): TargetResult[] {
  const checkers = checkersIn(results);
  return results.filter(
    (result) => result.baselineStatus === 'ok' && checkers.every((name) => result.scores[name]?.status === 'ok'),
  );
}

export function aggregate(results: TargetResult[], checker: Checker, commonOnly: boolean): Aggregate {
  const eligible = results.filter((result) => result.baselineStatus === 'ok');
  const scored = commonOnly ? commonCompleted(results) : eligible.filter((result) => result.scores[checker]?.status === 'ok');
  const sum = (field: 'exact' | 'fp' | 'fn' | 'mismatched' | 'spanless' | 'outOfProgram' | 'unmapped' | 'ms') =>
    scored.reduce((total, result) => total + (result.scores[checker]?.[field] ?? 0), 0);
  const exact = sum('exact');
  const fp = sum('fp');
  const fn = sum('fn');
  const fields: Partial<Record<MismatchField, number>> = {};
  for (const result of scored) {
    for (const [field, n] of Object.entries(result.scores[checker]?.fields ?? {}) as Array<[MismatchField, number]>) {
      fields[field] = (fields[field] ?? 0) + n;
    }
  }
  const precision = exact + fp > 0 ? exact / (exact + fp) : null;
  const recall = exact + fn > 0 ? exact / (exact + fn) : null;
  const f1 = precision !== null && recall !== null && precision + recall > 0 ? (2 * precision * recall) / (precision + recall) : null;
  return {
    targets: eligible.length,
    completed: eligible.filter((result) => result.scores[checker]?.status === 'ok').length,
    crashed: eligible.filter((result) => result.scores[checker]?.status === 'crash').length,
    timedOut: eligible.filter((result) => result.scores[checker]?.status === 'timeout').length,
    memoryLimited: eligible.filter((result) => result.scores[checker]?.status === 'memory-limit').length,
    noInput: eligible.filter((result) => result.scores[checker]?.status === 'no-input').length,
    exactTargets: scored.filter((result) => result.scores[checker]?.fp === 0 && result.scores[checker]?.fn === 0).length,
    exact,
    fp,
    fn,
    mismatched: sum('mismatched'),
    spanless: sum('spanless'),
    fields,
    outOfProgram: sum('outOfProgram'),
    unmapped: sum('unmapped'),
    precision,
    recall,
    f1,
    totalMs: sum('ms'),
  };
}

function topCodes(results: TargetResult[], checker: Checker, field: 'fpKeys' | 'fnKeys', limit = 12): Array<[string, number]> {
  const counts = new Map<string, number>();
  for (const result of results) {
    for (const key of result.scores[checker]?.[field] ?? []) {
      const code = key.slice(key.lastIndexOf(':') + 1);
      counts.set(code, (counts.get(code) ?? 0) + 1);
    }
  }
  return [...counts].sort((a, b) => b[1] - a[1]).slice(0, limit);
}

function configNotes(results: TargetResult[], checker: Checker): Array<[string, number]> {
  const counts = new Map<string, number>();
  for (const result of results) {
    for (const note of new Set(result.scores[checker]?.configNotes ?? [])) {
      counts.set(note, (counts.get(note) ?? 0) + 1);
    }
  }
  return [...counts].sort((a, b) => b[1] - a[1]);
}

function crashReasons(results: TargetResult[], checker: Checker, limit = 10): Array<[string, number]> {
  const counts = new Map<string, number>();
  for (const result of results) {
    const score = result.scores[checker];
    if (score && score.status !== 'ok') {
      const reason = `${score.status}: ${(score.reason ?? '').replace(/\(\d+\)/g, '').replace(/`[^`]*`/g, '`…`')}`.slice(0, 140);
      counts.set(reason, (counts.get(reason) ?? 0) + 1);
    }
  }
  return [...counts].sort((a, b) => b[1] - a[1]).slice(0, limit);
}

const percent = (value: number | null) => (value === null ? '—' : `${(value * 100).toFixed(1)}%`);

export function renderMarkdown(results: TargetResult[], meta: Record<string, string>): string {
  const checkers = checkersIn(results);
  const lines: string[] = [];
  lines.push(`# Diagnostic accuracy: ${checkers.join(' vs ')}`, '');
  lines.push(
    "Baseline: tsgo (TypeScript 7.0) CLI `--pretty false` output, with spans and related information taken from tsgo's API. A reported diagnostic is exact only when it agrees with a tsgo diagnostic on file, line, column, code, category, span and the complete message text, message chains included; anything else is a false positive, and the tsgo diagnostic it missed a false negative.",
    '',
  );
  for (const [key, value] of Object.entries(meta)) {
    lines.push(`- ${key}: ${value}`);
  }
  lines.push('');
  for (const tier of tierOrder) {
    const tierResults = results.filter((result) => result.tier === tier);
    if (tierResults.length === 0) {
      continue;
    }
    lines.push(`## ${tierHeadings[tier]}`, '');
    const invalid = tierResults.filter((result) => result.baselineStatus === 'invalid-config').length;
    if (invalid > 0) {
      lines.push(`${invalid} of ${tierResults.length} targets not scored: tsgo rejected their options (TS5xxx).`, '');
    }
    renderBaselineCoverage(lines, tierResults);
    for (const commonOnly of [true, false]) {
      lines.push(commonOnly
        ? '### Common completed set (targets every checker finished)'
        : '### Each checker over its own completed targets', '');
      lines.push('| checker | completed | crash | timeout | memory limit | no input | exact targets | exact | FP | FN | precision | recall | F1 | near misses | compared without tsgo span | out-of-program | unmapped | total time |');
      lines.push('|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|');
      for (const checker of checkers) {
        const agg = aggregate(tierResults, checker, commonOnly);
        const scoredTargets = commonOnly ? commonCompleted(tierResults).length : agg.completed;
        lines.push(
          `| ${checker} | ${agg.completed}/${agg.targets} | ${agg.crashed} | ${agg.timedOut} | ${agg.memoryLimited} | ${agg.noInput} | ${agg.exactTargets}/${scoredTargets} | ${agg.exact} | ${agg.fp} | ${agg.fn} | ${percent(agg.precision)} | ${percent(agg.recall)} | ${percent(agg.f1)} | ${agg.mismatched} | ${agg.spanless} | ${agg.outOfProgram} | ${agg.unmapped} | ${(agg.totalMs / 1000).toFixed(1)}s |`,
        );
      }
      lines.push('');
    }
    renderMismatches(lines, tierResults);
    for (const checker of checkers) {
      lines.push(`### ${checker}: top codes`, '');
      lines.push(`- FP: ${topCodes(tierResults, checker, 'fpKeys').map(([code, n]) => `${code}×${n}`).join(', ') || 'none'}`);
      lines.push(`- FN: ${topCodes(tierResults, checker, 'fnKeys').map(([code, n]) => `${code}×${n}`).join(', ') || 'none'}`);
      const notes = configNotes(tierResults, checker);
      if (notes.length > 0) {
        lines.push(`- Config translation gaps: ${notes.map(([note, n]) => `${note} ×${n}`).join(', ')}`);
      }
      const reasons = crashReasons(tierResults, checker);
      if (reasons.length > 0) {
        lines.push('- Failures:');
        for (const [reason, n] of reasons) lines.push(`  - ${n}× ${reason}`);
      }
      lines.push('');
    }
    if (tier === 'corpus') {
      const bolt = checkers.includes('bolt-ts');
      lines.push(`| corpus | tsgo | ${checkers.map((checker) => `${checker} exact/FP/FN`).join(' | ')} |${bolt ? ' bolt-ts unmapped |' : ''}`);
      lines.push(`|---|---|${checkers.map(() => '---|').join('')}${bolt ? '---|' : ''}`);
      for (const result of tierResults) {
        const cell = (checker: Checker) => {
          const score = result.scores[checker];
          return !score ? '—' : score.status !== 'ok' ? score.status : `${score.exact}/${score.fp}/${score.fn}`;
        };
        lines.push(`| ${result.name} | ${result.baselineStatus === 'ok' ? result.baseline : result.baselineStatus} | ${checkers.map(cell).join(' | ')} |${bolt ? ` ${result.scores['bolt-ts']?.unmapped ?? '—'} |` : ''}`);
      }
      lines.push('');
    }
  }
  const unmapped = new Map<string, number>();
  for (const result of results) {
    for (const message of result.scores['bolt-ts']?.unmappedMessages ?? []) {
      const shape = message.replace(/'[^']*'/g, "'…'");
      unmapped.set(shape, (unmapped.get(shape) ?? 0) + 1);
    }
  }
  if (unmapped.size > 0) {
    lines.push('## bolt-ts messages with no TS template (excluded from FP)', '');
    for (const [shape, n] of [...unmapped].sort((a, b) => b[1] - a[1]).slice(0, 20)) {
      lines.push(`- ${n}× ${shape}`);
    }
    lines.push('');
  }
  lines.push('## Caveats', '');
  lines.push('- The upstream tier is the neutral one for surge-ts and bolt-ts: TypeScript test cases with every case either checker uses in its own test suite removed (by name, option-suffixed name, and whitespace-insensitive content). Each case is turned into a project the way TypeScript\'s harness reads it; the first value of an option variation is used, and emit-only options are dropped because every tool runs `noEmit`.');
  lines.push('- ts-rust is a line-by-line port of typescript-go and ports its whole test suite, so no upstream case is held out from it. It is pinned to TypeScript 7.1.0-dev (see its version in the header), so where 7.1 changed behavior from the 7.0 baseline its output scores as FP/FN. It runs like the baseline: `--pretty false` text for the diagnostic set and its port of tsgo\'s API for spans.');
  lines.push('- Fixtures were written to drive surge-ts development against tsgo, so the fixture tier is biased toward surge-ts. A green fixture is parity on that fixture, not a compatibility claim.');
  lines.push('- The corpora are the projects surge-ts has burned its false positives down on, and three of them are checked through a surge-specific `tsconfig.surge.json`; the corpus tier is biased toward surge-ts as well.');
  lines.push('- bolt-ts prints no diagnostic codes; codes are recovered by matching its message text against the TypeScript 6 diagnostic table. A message it words differently from tsc is `unmapped` (not an FP), and the tsgo diagnostic it stood for still counts as an FN.');
  lines.push('- Each tool\'s output is read as printed. Columns are compared as printed (tsgo counts UTF-16 units, surge-ts code points, bolt-ts bytes); spans are converted to UTF-16 line/character from each tool\'s own offsets (tsgo\'s API, surge-ts\'s JSON byte spans, bolt-ts\'s column plus miette\'s underline). bolt-ts\'s span is unobservable for a label that spans lines, and its message is compared with whitespace runs collapsed because miette rewraps it. surge-ts runs twice, once for its JSON report and once for its tsc-style text, which alone shows the category.');
  lines.push('- A near miss shares file, line and code with a tsgo diagnostic but differs in another field; it counts as one FP and one FN, and the near-miss table says which fields differed. tsgo\'s related information is recorded, not compared.');
  lines.push('- bolt-ts cannot read `extends`, `files` or a directory `include`, so each config is resolved by TypeScript and handed to bolt-ts as explicit root files plus effective options in bolt-ts spelling. Options it has no field for (`paths`, `types`, `skipLibCheck`, …) and libs it does not ship are dropped and listed as config translation gaps. Per its README it also does not resolve `exports`/`imports` or `node_modules/@types`.');
  lines.push('- `memory limit` means the harness killed the process for exceeding its memory cap (`--maxMemory`, `--corpusMaxMemory`; physical footprint on macOS, resident size elsewhere); like crashes and timeouts it is counted, never scored.');
  lines.push('- `no input` means bolt-ts exited cleanly having loaded only its bundled lib files (its include glob matched no project source); such targets are not scored.');
  lines.push('- Diagnostics in files outside the tsgo program (`--listFilesOnly`) are counted as out-of-program, not FP. bolt-ts globs `.js` sources even without `allowJs`.');
  lines.push('');
  return lines.join('\n');
}

function renderBaselineCoverage(lines: string[], results: TargetResult[]): void {
  const scored = results.filter((result) => result.baselineStatus === 'ok');
  const total = scored.reduce((sum, result) => sum + result.baseline, 0);
  const sum = (field: 'located' | 'withSpan' | 'withRelated') =>
    scored.reduce((count, result) => count + (result.baselineSpans?.[field] ?? 0), 0);
  const located = sum('located');
  const withSpan = sum('withSpan');
  const failed = scored.filter((result) => result.baselineSpans?.api === 'failed').length;
  lines.push(
    `tsgo baseline: ${total} diagnostics over ${scored.length} scored targets, ${located} of them in a file; ${withSpan} of those (${percent(located ? withSpan / located : null)}) carry a span from tsgo's API, ${sum('withRelated')} carry related information (recorded, not compared).` +
      (failed ? ` The API run failed on ${failed} targets; their diagnostics are compared without a span.` : ''),
    '',
  );
}

function formatSnapshot(label: string, snap: MismatchExample['expected']): string {
  const location = snap.file ? `${snap.file}:${snap.line ?? '?'}:${snap.column ?? '?'}` : '(global)';
  return `${label} ${location} ${snap.category ?? '(no category)'} ${snap.code}${snap.span ? ` span ${snap.span}` : ''} ${JSON.stringify(snap.message)}`;
}

function renderMismatches(lines: string[], results: TargetResult[]): void {
  const checkers = checkersIn(results);
  const common = commonCompleted(results);
  if (common.length === 0) return;
  const totals = Object.fromEntries(checkers.map((checker) => [checker, aggregate(results, checker, true).fields])) as Record<Checker, Partial<Record<MismatchField, number>>>;
  const used = (Object.keys(mismatchMeanings) as MismatchField[]).filter((field) => checkers.some((checker) => totals[checker][field]));
  if (used.length > 0) {
    lines.push('### Near misses by field (common completed set)', '');
    lines.push('A near miss that differs in several fields counts once per field.', '');
    lines.push(`| field | ${checkers.join(' | ')} | meaning |`);
    lines.push(`|---|${checkers.map(() => '---').join('|')}|---|`);
    for (const field of used) {
      lines.push(`| ${field} | ${checkers.map((checker) => totals[checker][field] ?? 0).join(' | ')} | ${mismatchMeanings[field]} |`);
    }
    lines.push('');
  }
  for (const checker of checkers) {
    const scores = common.map((result) => ({ result, score: result.scores[checker] as CheckerScore }));
    const observed = scores.reduce((sum, { score }) => sum + (score.observed?.diagnostics ?? 0), 0);
    const category = scores.reduce((sum, { score }) => sum + (score.observed?.category ?? 0), 0);
    const span = scores.reduce((sum, { score }) => sum + (score.observed?.span ?? 0), 0);
    const notes = new Map<string, number>();
    for (const { score } of scores) {
      for (const [note, n] of Object.entries(score.spanNotes ?? {})) notes.set(note, (notes.get(note) ?? 0) + n);
    }
    lines.push(`#### ${checker}: what its output shows`, '');
    lines.push(`- ${observed} scored diagnostics: category shown for ${category} (${percent(observed ? category / observed : null)}), span recovered for ${span} (${percent(observed ? span / observed : null)}).`);
    if (notes.size > 0) {
      lines.push(`- No span: ${[...notes].sort((a, b) => b[1] - a[1]).map(([note, n]) => `${note} ×${n}`).join('; ')}.`);
    }
    const examples = scores.flatMap(({ result, score }) => score.examples.map((example) => ({ ...example, target: result.name }))).slice(0, 12);
    if (examples.length > 0) {
      lines.push('- Examples (tsgo first):');
      for (const example of examples) {
        lines.push(`  - ${example.fields.join(', ')} in ${example.target}: ${formatSnapshot('tsgo', example.expected)} / ${formatSnapshot(checker, example.actual)}`);
      }
    }
    lines.push('');
  }
}

const checkerColors: Record<Checker, string> = { 'surge-ts': '#de7a4a', 'bolt-ts': '#7c5cc4', 'ts-rust': '#2a8a7e' };
const goodPill: DriftStyle = { bg: '#e6f4ea', fg: '#137333' };
const badPill: DriftStyle = { bg: '#fce8e6', fg: '#c5221f' };
const tierOrder: Tier[] = ['upstream', 'fixture', 'corpus'];
const tierTitles: Record<Tier, string> = { upstream: 'Upstream held-out', fixture: 'Fixtures (surge-biased)', corpus: 'Corpora (surge-biased)' };
const tierHeadings: Record<Tier, string> = {
  upstream: 'Upstream held-out cases (neutral: in neither checker\'s test suite)',
  fixture: 'Fixtures (tests/compat-projects; written for surge-ts, biased toward it)',
  corpus: 'Real-project corpora (surge-ts was tuned on these; biased toward it)',
};

function barRow(label: string, color: string, value: number, valueText: string, tooltip: string, pill?: { text: string; style: DriftStyle }): PanelRow {
  return {
    label,
    color,
    value,
    min: value,
    max: value,
    valueText,
    ratioText: null,
    ratioGood: false,
    drift: pill?.text ?? null,
    pillStyle: pill?.style,
    tooltip,
  };
}

function failurePill(agg: Aggregate): { text: string; style: DriftStyle } {
  const parts = [
    agg.crashed ? `${agg.crashed} crash` : '',
    agg.timedOut ? `${agg.timedOut} timeout` : '',
    agg.memoryLimited ? `${agg.memoryLimited} memory limit` : '',
    agg.noInput ? `${agg.noInput} no input` : '',
  ].filter(Boolean);
  return parts.length ? { text: parts.join(' · '), style: badPill } : { text: 'all completed', style: goodPill };
}

export function buildCheckerPanels(results: TargetResult[]): Panel[] {
  const checkers = checkersIn(results);
  const tiers = tierOrder.filter((tier) => results.some((result) => result.tier === tier));
  const completion: PanelGroup[] = [];
  const accuracy: PanelGroup[] = [];
  const errors: PanelGroup[] = [];
  let maxTargets = 0;
  let maxErrors = 0;
  for (const tier of tiers) {
    const tierResults = results.filter((result) => result.tier === tier);
    const common = commonCompleted(tierResults).length;
    completion.push({
      project: `${tierTitles[tier]} (${tierResults.length} targets)`,
      rows: checkers.map((checker) => {
        const agg = aggregate(tierResults, checker, false);
        maxTargets = Math.max(maxTargets, agg.targets);
        return barRow(checker, checkerColors[checker], agg.completed, `${agg.completed}/${agg.targets}`,
          `${checker}: completed ${agg.completed} of ${agg.targets}`, failurePill(agg));
      }),
    });
    if (common === 0) {
      continue;
    }
    const group = `${tierTitles[tier]} (${common} targets every checker completed)`;
    const accuracyRows: PanelRow[] = [];
    const errorRows: PanelRow[] = [];
    for (const checker of checkers) {
      const agg = aggregate(tierResults, checker, true);
      for (const [metric, value] of [['precision', agg.precision], ['recall', agg.recall], ['F1', agg.f1]] as const) {
        accuracyRows.push(barRow(`${checker} ${metric}`, checkerColors[checker], (value ?? 0) * 100, percent(value),
          `${checker} ${metric}: ${percent(value)} (exact ${agg.exact}, FP ${agg.fp}, FN ${agg.fn})`));
      }
      maxErrors = Math.max(maxErrors, agg.fp, agg.fn);
      errorRows.push(barRow(`${checker} FP`, checkerColors[checker], agg.fp, String(agg.fp), `${checker}: ${agg.fp} false positives`,
        agg.unmapped ? { text: `${agg.unmapped} unmapped`, style: { bg: '#eceff1', fg: '#546e7a' } } : undefined));
      errorRows.push(barRow(`${checker} FN`, checkerColors[checker], agg.fn, String(agg.fn), `${checker}: ${agg.fn} false negatives`));
    }
    accuracy.push({ project: group, rows: accuracyRows });
    errors.push({ project: group, rows: errorRows });
  }

  const panels: Panel[] = [
    { title: 'Targets completed (higher is better)', groups: completion, maxValue: maxTargets, omitRatioColumn: true, tickFormat: (v) => String(v) },
  ];
  if (accuracy.length > 0) {
    panels.push({ title: 'Accuracy vs tsgo on targets every checker completed (higher is better)', groups: accuracy, maxValue: 100, omitRatioColumn: true, tickFormat: (v) => `${v}%` });
    panels.push({ title: 'False positives / false negatives vs tsgo (lower is better)', groups: errors, maxValue: maxErrors, omitRatioColumn: true, tickFormat: (v) => String(v) });
  }

  const corpora = results.filter((result) => result.tier === 'corpus');
  if (corpora.length > 0) {
    let maxCorpus = 0;
    const groups = corpora.map((result) => ({
      project: `${result.name} (tsgo: ${result.baselineStatus === 'ok' ? result.baseline : result.baselineStatus})`,
      rows: checkers.map((checker) => {
        const score = result.scores[checker];
        if (!score || score.status !== 'ok') {
          const status = score?.status ?? 'not run';
          return barRow(checker, checkerColors[checker], 0, '—', `${checker}: ${status}${score?.reason ? ` (${score.reason})` : ''}`, { text: status, style: badPill });
        }
        maxCorpus = Math.max(maxCorpus, score.fp + score.fn);
        return barRow(checker, checkerColors[checker], score.fp + score.fn, `${score.fp}/${score.fn}`,
          `${checker}: exact ${score.exact}, FP ${score.fp}, FN ${score.fn}`,
          score.fp + score.fn === 0 ? { text: 'exact', style: goodPill } : undefined);
      }),
    }));
    panels.push({ title: 'Per corpus: FP + FN vs tsgo (label FP/FN, lower is better)', groups, maxValue: maxCorpus, omitRatioColumn: true, tickFormat: (v) => String(v) });
  }
  return panels;
}

export function renderCheckerSvg(results: TargetResult[], meta: Record<string, string>): string {
  const checkers = checkersIn(results);
  return renderPanelsSvg({
    title: `Diagnostic accuracy: ${checkers.join(' vs ')}`,
    subtitleParts: [meta.date?.replace('T', ' ').replace(/\.\d+Z$/, 'Z'), `tsgo@${meta.tsgo}`, meta.platform].filter(Boolean),
    panels: buildCheckerPanels(results),
    footer: `Baseline tsgo · exact on file, line, column, code, category, span and message · crashed, timed-out and no-input targets are not scored · only the upstream held-out tier is neutral; fixtures and corpora are biased toward surge-ts`,
  });
}

// Converts the subset of Markdown renderMarkdown emits: headings, pipe
// tables, bullet lists (one nesting level), paragraphs and inline code.
function markdownToHtml(markdown: string): string {
  const inline = (text: string) =>
    escapeHtml(text).replace(/`([^`]+)`/g, '<code>$1</code>');
  const out: string[] = [];
  const lines = markdown.split('\n');
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index];
    const heading = /^(#{1,3}) (.*)$/.exec(line);
    if (heading) {
      out.push(`<h${heading[1].length + 1}>${inline(heading[2])}</h${heading[1].length + 1}>`);
    } else if (line.startsWith('|')) {
      const rows: string[][] = [];
      while (index < lines.length && lines[index].startsWith('|')) {
        const cells = lines[index].slice(1, -1).split('|').map((cell) => cell.trim());
        if (!cells.every((cell) => /^-+$/.test(cell))) rows.push(cells);
        index++;
      }
      index--;
      const [head, ...body] = rows;
      out.push(`<table><thead><tr>${head.map((cell) => `<th>${inline(cell)}</th>`).join('')}</tr></thead><tbody>${body
        .map((row) => `<tr>${row.map((cell, i) => `<td${i > 0 ? ' class="num"' : ''}>${inline(cell)}</td>`).join('')}</tr>`)
        .join('')}</tbody></table>`);
    } else if (/^\s*- /.test(line)) {
      out.push('<ul>');
      while (index < lines.length && /^\s*- /.test(lines[index])) {
        const nested = lines[index].startsWith('  ');
        out.push(`<li${nested ? ' style="margin-left:1.5em"' : ''}>${inline(lines[index].replace(/^\s*- /, ''))}</li>`);
        index++;
      }
      index--;
      out.push('</ul>');
    } else if (line.trim()) {
      out.push(`<p>${inline(line)}</p>`);
    }
  }
  return out.join('\n');
}

export function renderCheckerHtml(results: TargetResult[], meta: Record<string, string>, markdown: string): string {
  // The Markdown report repeats the title and meta list the page chrome already shows.
  const details = markdown.replace(/^# .*\n+/, '').replace(/^Baseline:.*\n+(- .*\n)+\n?/, '');
  return renderReportPage({
    title: `Diagnostic accuracy: ${checkersIn(results).join(' vs ')}`,
    subtitle: 'Exact matches, false positives and false negatives against tsgo (TypeScript 7.0); a diagnostic matches only when file, line, column, code, category, span and full message all agree.',
    metaHtml: metaGridHtml(Object.entries(meta)),
    body: `<div class="chart-wrap">${renderCheckerSvg(results, meta)}</div>\n${markdownToHtml(details)}`,
    disclaimerHtml: 'Fixtures were written to drive surge-ts development, so the fixture tier favors surge-ts; neither tier is a compatibility claim. bolt-ts codes are recovered from message text, and bolt-ts does not read several tsconfig features surge-ts and tsgo honor. See scripts/bench/README.md.',
  });
}

function collectUpstreamTargets(args: ParsedArgs, notes: Record<string, string>): Target[] {
  const pool = listUpstreamCases(upstreamCaseRoots);
  if (pool.length === 0) {
    throw new Error(`No upstream test cases under ${upstreamCaseRoots.map((root) => path.relative(workspaceRoot, root)).join(' or ')}; see scripts/bench/README.md.`);
  }
  const names = new Set(pool.map((source) => source.name));
  const boltRepo = path.resolve(path.dirname(args.boltBin), '..', '..');
  const suites = [boltTestSuite(boltRepo, names), surgeTestSuite(workspaceRoot)];
  const selection = selectHeldOutCases(pool, suites, args.upstreamLimit);
  const outDir = path.join(args.out, 'upstream-cases');
  const skipped = new Map<string, number>();
  const targets: Target[] = [];
  for (const source of selection.cases) {
    const materialized = materializeCase(source, outDir, tsDiagnosticTablePath);
    if ('skip' in materialized) {
      const reason = materialized.skip.replace(/: .*$/, '');
      skipped.set(reason, (skipped.get(reason) ?? 0) + 1);
      continue;
    }
    targets.push({ name: source.name, tier: 'upstream', tsconfig: materialized.tsconfig });
  }
  const excluded = Object.entries(selection.excluded).map(([label, n]) => `${n} in ${label}'s tests`).join(', ');
  const unusable = [...skipped].map(([reason, n]) => `${n} ${reason}`).join(', ');
  notes['upstream cases'] =
    `${selection.pool} in pool, excluded ${excluded}; ${args.upstreamLimit > 0 ? `hash-sampled ${selection.sampled}` : `all ${selection.sampled} held out`}` +
    `${unusable ? `, not materializable: ${unusable}` : ''}`;
  return targets;
}

function collectTargets(args: ParsedArgs, notes: Record<string, string>): Target[] {
  const targets: Target[] = [];
  if (args.upstream) {
    targets.push(...collectUpstreamTargets(args, notes));
  }
  if (args.fixtures) {
    for (const target of discoverProjectTargets('tests/compat-projects')) {
      targets.push({ name: target.name, tier: 'fixture', tsconfig: target.resolvedPath });
    }
  }
  if (args.corpora) {
    for (const [name, relative] of Object.entries(corpusTargets)) {
      const tsconfig = path.join(workspaceRoot, relative);
      if (existsSync(tsconfig)) {
        targets.push({ name, tier: 'corpus', tsconfig });
      } else {
        console.log(`  corpus ${name} skipped (not provisioned: ${relative})`);
      }
    }
  }
  for (const project of args.projects) {
    const tsconfig = resolveProjectPresetOrPath(project);
    targets.push({ name: path.relative(workspaceRoot, tsconfig), tier: 'fixture', tsconfig });
  }
  const filtered = args.filters.length
    ? targets.filter((target) => args.filters.some((filter) => target.name.includes(filter)))
    : targets;
  return filtered;
}

function parseArgs(argv: string[]): ParsedArgs {
  const parsed: ParsedArgs = {
    upstream: false,
    upstreamLimit: 1000,
    fixtures: false,
    corpora: false,
    projects: [],
    filters: [],
    jobs: Math.max(1, Math.min(8, Math.floor(os.availableParallelism() / 2))),
    timeoutMs: 60_000,
    corpusTimeoutMs: 600_000,
    maxMemoryMb: 2048,
    corpusMaxMemoryMb: 6144,
    out: path.join(workspaceRoot, '.bench', 'checkers'),
    boltBin: process.env.BOLT_TS_BIN ?? defaultBoltBin,
    surgeBin: process.env.SURGE_TS_BIN ?? defaultSurgeBin,
    tsRustBin: process.env.TS_RUST_BIN ?? defaultTsRustBin(),
    baselineCache: true,
    checkers: [...allCheckers],
  };
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index];
    const value = () => {
      const next = argv[++index];
      if (next === undefined) throw new Error(`${arg} expects a value`);
      return next;
    };
    if (arg === '--') continue;
    else if (arg === '--upstream') parsed.upstream = true;
    else if (arg === '--upstreamLimit') parsed.upstreamLimit = Number(value());
    else if (arg === '--fixtures') parsed.fixtures = true;
    else if (arg === '--corpora') parsed.corpora = true;
    else if (arg === '--all') parsed.upstream = parsed.fixtures = parsed.corpora = true;
    else if (arg === '--project') parsed.projects.push(value());
    else if (arg === '--filter') parsed.filters.push(value());
    else if (arg === '--jobs') parsed.jobs = Number(value());
    else if (arg === '--timeout') parsed.timeoutMs = Number(value()) * 1000;
    else if (arg === '--corpusTimeout') parsed.corpusTimeoutMs = Number(value()) * 1000;
    else if (arg === '--maxMemory') parsed.maxMemoryMb = Number(value());
    else if (arg === '--corpusMaxMemory') parsed.corpusMaxMemoryMb = Number(value());
    else if (arg === '--out') parsed.out = path.resolve(value());
    else if (arg === '--boltBin') parsed.boltBin = path.resolve(value());
    else if (arg === '--surgeBin') parsed.surgeBin = path.resolve(value());
    else if (arg === '--tsRustBin') parsed.tsRustBin = path.resolve(value());
    else if (arg === '--fromJson') parsed.fromJson = path.resolve(value());
    else if (arg === '--noBaselineCache') parsed.baselineCache = false;
    else if (arg === '--checkers') parsed.checkers = parseCheckers(value());
    else if (arg === '--skipBolt') parsed.checkers = parsed.checkers.filter((checker) => checker !== 'bolt-ts');
    else if (arg === '--skipTsRust') parsed.checkers = parsed.checkers.filter((checker) => checker !== 'ts-rust');
    else throw new Error(`unknown argument: ${arg}\n${usage()}`);
  }
  // Concurrent targets can each grow to the cap; keep their sum within half of
  // physical memory so the other half stays for the OS and the parent.
  const memoryBoundJobs = Math.max(1, Math.floor(os.totalmem() / 2 / (parsed.maxMemoryMb * 1024 * 1024)));
  if (parsed.jobs > memoryBoundJobs) {
    console.log(`  --jobs ${parsed.jobs} lowered to ${memoryBoundJobs}: ${parsed.jobs} × ${parsed.maxMemoryMb} MB exceeds half of physical memory`);
    parsed.jobs = memoryBoundJobs;
  }
  if (!parsed.upstream && !parsed.fixtures && !parsed.corpora && parsed.projects.length === 0) {
    parsed.upstream = true;
  }
  if (parsed.checkers.length === 0) throw new Error(`no checker selected\n${usage()}`);
  return parsed;
}

function parseCheckers(list: string): Checker[] {
  const names = list.split(',').map((name) => name.trim()).filter(Boolean);
  for (const name of names) {
    if (!allCheckers.includes(name as Checker)) throw new Error(`unknown checker: ${name} (expected ${allCheckers.join(', ')})`);
  }
  return allCheckers.filter((checker) => names.includes(checker));
}

function usage(): string {
  return [
    'Usage: pnpm run bench:checkers -- [--upstream] [--fixtures] [--corpora] [--all] [--project <tsconfig>]',
    '         [--upstreamLimit <n>] [--filter <substring>] [--jobs <n>] [--timeout <s>] [--corpusTimeout <s>]',
    '         [--maxMemory <MB>] [--corpusMaxMemory <MB>]',
    '         [--out <dir>] [--boltBin <path>] [--surgeBin <path>] [--tsRustBin <path>] [--fromJson <report.json>] [--noBaselineCache]',
    '         [--checkers <surge-ts,bolt-ts,ts-rust>] [--skipBolt] [--skipTsRust]',
    '  Binaries default to ../bolt-ts/target/release/bolt_ts_compiler, target/release/surge and the native tsc',
    '  of the tsc-rs devDependency (override with BOLT_TS_BIN / SURGE_TS_BIN / TS_RUST_BIN).',
    '  --upstream (the default) is the held-out tier; --upstreamLimit 0 runs every held-out case (default 1000).',
    '  Every spawned tool is killed past --maxMemory (default 2048 MB; corpora --corpusMaxMemory, default 6144 MB).',
    '  --checkers picks which checkers run (default all three); --skipBolt / --skipTsRust drop one.',
    '  bolt-ts\'s test suite shapes the held-out selection even when bolt-ts does not run.',
  ].join('\n');
}

function binaryStamp(bin: string): string {
  return `built ${statSync(bin).mtime.toISOString()}`;
}

function gitDescribe(cwd: string): string {
  try {
    const head = spawnSyncText('git', ['rev-parse', '--short', 'HEAD'], cwd);
    const dirty = spawnSyncText('git', ['status', '--porcelain', '--untracked-files=no'], cwd) ? ' (dirty tree)' : '';
    return `${head}${dirty}`;
  } catch {
    return 'unknown';
  }
}

function spawnSyncText(command: string, args: string[], cwd: string): string {
  return spawnSync(command, args, { cwd, encoding: 'utf8' }).stdout.trim();
}

function writeReports(out: string, results: TargetResult[], meta: Record<string, string>): string {
  const markdown = renderMarkdown(results, meta);
  mkdirSync(out, { recursive: true });
  writeFileSync(path.join(out, 'report.json'), `${JSON.stringify({ meta, results }, null, 2)}\n`);
  writeFileSync(path.join(out, 'report.md'), markdown);
  writeFileSync(path.join(out, 'report.svg'), `${renderCheckerSvg(results, meta)}\n`);
  writeFileSync(path.join(out, 'report.html'), `${renderCheckerHtml(results, meta, markdown)}\n`);
  return markdown;
}

async function main(argv = process.argv.slice(2)): Promise<number> {
  const args = parseArgs(argv);
  if (args.fromJson) {
    const { meta, results } = JSON.parse(readFileSync(args.fromJson, 'utf8')) as { meta: Record<string, string>; results: TargetResult[] };
    if (results.some((result) => Object.values(result.scores).some((score) => score && !('exact' in score)))) {
      console.error(`${args.fromJson} was scored on file:line:code, which this harness no longer renders; rerun the benchmark.`);
      return 1;
    }
    const out = argv.includes('--out') ? args.out : path.dirname(args.fromJson);
    writeReports(out, results, meta);
    console.log(`Rendered report.{md,svg,html} to ${path.relative(workspaceRoot, out) || '.'}`);
    return 0;
  }
  const { checkers } = args;
  const binaries: Record<Checker, [string, string]> = {
    'bolt-ts': [args.boltBin, 'Clone https://github.com/bvanjoi/bolt-ts next to this checkout and run `cargo build --release -p bolt_ts_compiler`, or set BOLT_TS_BIN.'],
    'surge-ts': [args.surgeBin, 'Run `cargo build --release -p surge-ts-cli`, or set SURGE_TS_BIN.'],
    'ts-rust': [args.tsRustBin, 'Run `pnpm install` (the tsc-rs devDependency), or set TS_RUST_BIN.'],
  };
  for (const checker of checkers) {
    const [bin, hint] = binaries[checker];
    if (!existsSync(bin)) {
      console.error(`Missing ${checker} binary: ${bin}`);
      console.error(hint);
      return 1;
    }
  }
  const selectionNotes: Record<string, string> = {};
  const targets = collectTargets(args, selectionNotes);
  if (targets.length === 0) {
    console.error('No targets selected.');
    return 1;
  }
  loadMessageTemplates();

  const fixtures = targets.filter((target) => target.tier !== 'corpus');
  const corpora = targets.filter((target) => target.tier === 'corpus');
  let done = 0;
  const report = (result: TargetResult) => {
    done++;
    const cell = (checker: Checker) => {
      const score = result.scores[checker];
      return !score ? '-' : score.status !== 'ok' ? score.status : `fp=${score.fp} fn=${score.fn}`;
    };
    console.log(`[${done}/${targets.length}] ${result.name}  tsgo=${result.baselineStatus === 'ok' ? result.baseline : result.baselineStatus}  ${checkers.map((checker) => `${checker} ${cell(checker)}`).join('  ')}`);
    return result;
  };
  const results = [
    ...(await runPool(fixtures, args.jobs, async (target) => report(await evaluateTarget(target, args)))),
    // Corpora run one at a time: tsgo and surge-ts both parallelize internally,
    // and concurrent corpus checks have taken this machine down before.
    ...(await runPool(corpora, 1, async (target) => report(await evaluateTarget(target, args)))),
  ];

  const meta: Record<string, string> = {
    date: new Date().toISOString(),
    checkers: checkers.join(', '),
    ...(checkers.includes('surge-ts') ? { 'surge-ts': `${args.surgeBin} (${binaryStamp(args.surgeBin)}), tree ${gitDescribe(workspaceRoot)}` } : {}),
    ...(checkers.includes('bolt-ts')
      ? { 'bolt-ts': `${args.boltBin} (${binaryStamp(args.boltBin)}), tree ${gitDescribe(path.resolve(path.dirname(args.boltBin), '..', '..'))}` }
      : {}),
    ...(checkers.includes('ts-rust') ? { 'ts-rust': `${args.tsRustBin} (${spawnSyncText(args.tsRustBin, ['--version'], workspaceRoot)}, tsc-rs ${readPackageVersion('tsc-rs')})` } : {}),
    tsgo: readPackageVersion('typescript'),
    'code table': `typescript ${readPackageVersion('typescript-6')} (${loadMessageTemplates().length} error templates)`,
    platform: `${process.platform}-${process.arch}, ${os.availableParallelism()} cores`,
    ...selectionNotes,
  };
  const markdown = writeReports(args.out, results, meta);
  console.log(`\n${markdown}`);
  console.log(`Wrote report.{md,json,svg,html} to ${path.relative(workspaceRoot, args.out) || '.'}`);
  return 0;
}

function readPackageVersion(name: string): string {
  try {
    return (JSON.parse(readFileSync(path.join(workspaceRoot, 'node_modules', name, 'package.json'), 'utf8')) as { version: string }).version;
  } catch {
    return 'unknown';
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then((code) => process.exit(code), (error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  });
}
