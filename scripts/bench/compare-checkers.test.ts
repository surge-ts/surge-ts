import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { expect, test } from 'vitest';

import type { NormalizedDiagnostic } from '../oracle/compare-tsc.js';
import {
  aggregate,
  baselineCacheKey,
  codeForMessage,
  compileTemplate,
  loadMessageTemplates,
  parseBoltOutput,
  renderCheckerSvg,
  runProcess,
  scoreDiagnostics,
  translateConfigForBolt,
  type TargetResult,
} from './compare-checkers.js';

const boltOutput = [
  "  × Type 'number[]' is not assignable to type 'Iterable<string, any, any>'.",
  '    ╭─[src/index.ts:15:14]',
  ' 15 │ wrongElement',
  '    · ────────────',
  '    ╰────',
  '',
  "  × Non-abstract class 'MissingGeneric' does not implement inherited abstract",
  "  │ member 'handle' from class 'Generic<string>'.",
  '    ╭─[src/index.ts:54:8]',
  ' 54 │ class MissingGeneric extends Generic<string> {}',
  '    ╰────',
  '',
  'Advice: ',
  "  ☞ Class 'Later' is defined here",
  '    ╭─[src/index.ts:18:7]',
  '    ╰────',
  '',
  "  × Cannot find name 'Iterator'.",
  "  [Failed to read contents for label (offset: 71948, length: 7): OutOfBounds]",
  '',
  'Files: 58',
  'Types: 287',
  'Time cost: 17ms',
].join('\n');

test('parses miette reports, joining wrapped messages and skipping advice', () => {
  expect(parseBoltOutput(boltOutput)).toStrictEqual([
    {
      message: "Type 'number[]' is not assignable to type 'Iterable<string, any, any>'.",
      fileName: 'src/index.ts',
      line: 15,
      column: 14,
    },
    {
      message: "Non-abstract class 'MissingGeneric' does not implement inherited abstract member 'handle' from class 'Generic<string>'.",
      fileName: 'src/index.ts',
      line: 54,
      column: 8,
    },
    { message: "Cannot find name 'Iterator'.", fileName: '' },
  ]);
});

test('recovers codes from the TypeScript diagnostic table', () => {
  const templates = loadMessageTemplates();
  expect(
    codeForMessage("Type 'number' is not assignable to type 'string'.", templates),
  ).toBe('TS2322');
  expect(codeForMessage("Cannot find name 'foo'.", templates)).toBe('TS2304');
  expect(codeForMessage("Cannot find name 'foo'. Did you mean 'for'?", templates)).toBe('TS2552');
  expect(codeForMessage('Declarations must be initialized.', templates)).toBe(null);
});

test('a template with a double space still matches single-spaced text', () => {
  const template = compileTemplate('TS2403', "Subsequent variable declarations must have the same type.  Variable '{0}' must be of type '{1}'.");
  expect(
    codeForMessage("Subsequent variable declarations must have the same type. Variable 'x' must be of type 'string'.", [template]),
  ).toBe('TS2403');
});

const diag = (fileName: string, line: number, code: string): NormalizedDiagnostic => ({
  source: 'typescript',
  fileName,
  line,
  code,
});

test('scores the file:line:code multiset', () => {
  const baseline = [diag('a.ts', 1, 'TS2322'), diag('a.ts', 1, 'TS2322'), diag('a.ts', 3, 'TS2304')];
  const reported = [diag('a.ts', 1, 'TS2322'), diag('a.ts', 3, 'TS2552'), diag('b.ts', 9, 'TS2322')];
  const score = scoreDiagnostics(baseline, reported);
  expect(score.tp).toBe(1);
  expect(score.fpKeys.sort()).toStrictEqual(['a.ts:3:TS2552', 'b.ts:9:TS2322']);
  expect(score.fnKeys.sort()).toStrictEqual(['a.ts:1:TS2322', 'a.ts:3:TS2304']);
});

test('the strict key also requires the column and the message', () => {
  const at = (column: number, message: string, source: 'typescript' | 'surge-ts' = 'typescript') => ({
    source,
    fileName: 'a.ts',
    line: 1,
    column,
    code: 'TS2322',
    message,
  });
  const baseline = [at(5, "Type 'string' is not assignable to type 'number'."), at(9, 'm')];
  const reported = [
    at(5, "Type 'string' is not assignable to type 'number'.", 'surge-ts'),
    at(8, 'm', 'surge-ts'),
  ];
  expect(scoreDiagnostics(baseline, reported).tp).toBe(2);
  const strict = scoreDiagnostics(baseline, reported, 'strict');
  expect(strict.tp).toBe(1);
  expect(strict.fpKeys).toStrictEqual(['a.ts:1:8:"m":TS2322']);
  expect(strict.fnKeys).toStrictEqual(['a.ts:1:9:"m":TS2322']);
});

test('the common set drops a target either checker failed', () => {
  const score = (fp: number) => ({
    status: 'ok' as const, ms: 1, reported: fp, tp: 1, fp, fn: 0,
    outOfProgram: 0, unmapped: 0, fpKeys: [], fnKeys: [], unmappedMessages: [],
  });
  const results: TargetResult[] = [
    { name: 'a', tier: 'fixture', tsconfig: 'a', baselineStatus: 'ok', baseline: 1, scores: { 'surge-ts': score(0), 'bolt-ts': score(2) } },
    { name: 'b', tier: 'fixture', tsconfig: 'b', baselineStatus: 'ok', baseline: 1, scores: { 'surge-ts': score(5), 'bolt-ts': { ...score(0), status: 'crash' } } },
  ];
  expect(aggregate(results, 'surge-ts', true).fp).toBe(0);
  expect(aggregate(results, 'surge-ts', false).fp).toBe(5);
  expect(aggregate(results, 'bolt-ts', false).crashed).toBe(1);
});

test('maps one-to-one bolt-ts rewordings', () => {
  const templates = loadMessageTemplates();
  expect(codeForMessage("Property 'b' is missing.", templates)).toBe('TS2741');
  expect(codeForMessage("Generic type 'Box' requires 1 type argument.", templates)).toBe('TS2314');
});

test('renders a chart with a failure pill for a checker that crashed', () => {
  const score = { status: 'ok' as const, ms: 1, reported: 1, tp: 1, fp: 0, fn: 0, outOfProgram: 0, unmapped: 0, fpKeys: [], fnKeys: [], unmappedMessages: [] };
  const results: TargetResult[] = [
    { name: 'a', tier: 'fixture', tsconfig: 'a', baselineStatus: 'ok', baseline: 1, scores: { 'surge-ts': score, 'bolt-ts': score } },
    { name: 'b', tier: 'fixture', tsconfig: 'b', baselineStatus: 'ok', baseline: 1, scores: { 'surge-ts': score, 'bolt-ts': { ...score, status: 'crash' } } },
  ];
  const svg = renderCheckerSvg(results, { date: '2026-09-18T00:00:00.000Z', tsgo: '7.0.2', platform: 'test' });
  expect(svg.startsWith('<svg')).toBeTruthy();
  expect(svg).toContain('1 crash');
  expect(svg).toContain('1 targets both completed');
});

const fixture = (name: string) =>
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../tests/compat-projects', name, 'tsconfig.json');

test('hands bolt-ts the program root files and effective options', () => {
  const { config } = translateConfigForBolt(fixture('binding-pattern-parameters-basic'));
  expect(config.include.map((file) => path.basename(file))).toStrictEqual(['index.ts']);
  expect(config.compilerOptions.strictNullChecks).toBe(true);
  expect(config.compilerOptions.target).toBe('ES2025');
});

test('respells lib names the way bolt-ts deserializes them', () => {
  const { config } = translateConfigForBolt(fixture('default-lib-lib-option-dom-basic'));
  expect((config.compilerOptions.lib as string[])).toContain('dom');
});

test('a process past the memory cap is killed and flagged', async () => {
  const hog = 'const keep = []; setInterval(() => keep.push(Buffer.alloc(64 * 1024 * 1024, 1)), 10);';
  const output = await runProcess(process.execPath, ['-e', hog], {
    cwd: process.cwd(),
    timeoutMs: 20_000,
    maxMemoryBytes: 256 * 1024 * 1024,
  });
  expect(output.memoryExceeded).toBe(true);
  expect(output.timedOut).toBe(false);
  expect(output.ms < 10_000, `took ${output.ms}ms`).toBeTruthy();
});

test('the baseline cache key follows case content, location, and tsgo version', () => {
  const root = mkdtempSync(path.join(os.tmpdir(), 'baseline-key-'));
  try {
    const makeCase = (name: string, source: string) => {
      const dir = path.join(root, name);
      mkdirSync(path.join(dir, 'sub'), { recursive: true });
      writeFileSync(path.join(dir, 'tsconfig.json'), '{}');
      writeFileSync(path.join(dir, 'sub', 'a.ts'), source);
      return dir;
    };
    const one = makeCase('one', 'let x = 1;');
    const key = baselineCacheKey(one, '7.0.2');
    expect(baselineCacheKey(one, '7.0.2')).toBe(key);
    expect(baselineCacheKey(one, '7.0.3')).not.toBe(key);
    expect(baselineCacheKey(makeCase('two', 'let x = 1;'), '7.0.2')).not.toBe(key);
    writeFileSync(path.join(one, 'sub', 'a.ts'), 'let x = 2;');
    expect(baselineCacheKey(one, '7.0.2')).not.toBe(key);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
