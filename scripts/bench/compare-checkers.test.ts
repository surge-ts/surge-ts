import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { expect, test } from 'vitest';

import {
  aggregate,
  baselineCacheKey,
  codeForMessage,
  compileTemplate,
  loadMessageTemplates,
  renderCheckerSvg,
  renderMarkdown,
  runProcess,
  translateConfigForBolt,
  type CheckerScore,
  type TargetResult,
} from './compare-checkers.js';

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

const okScore = (overrides: Partial<CheckerScore> = {}): CheckerScore => ({
  status: 'ok', ms: 1, reported: 1, exact: 1, fp: 0, fn: 0, mismatched: 0, fields: {}, spanless: 0,
  fpKeys: [], fnKeys: [], examples: [], outOfProgram: 0, unmapped: 0, unmappedMessages: [],
  ...overrides,
});

test('the common set drops a target either checker failed', () => {
  const results: TargetResult[] = [
    { name: 'a', tier: 'fixture', tsconfig: 'a', baselineStatus: 'ok', baseline: 1, scores: { 'surge-ts': okScore(), 'bolt-ts': okScore({ fp: 2 }) } },
    { name: 'b', tier: 'fixture', tsconfig: 'b', baselineStatus: 'ok', baseline: 1, scores: { 'surge-ts': okScore({ fp: 5 }), 'bolt-ts': okScore({ status: 'crash' }) } },
  ];
  expect(aggregate(results, 'surge-ts', true).fp).toBe(0);
  expect(aggregate(results, 'surge-ts', false).fp).toBe(5);
  expect(aggregate(results, 'bolt-ts', false).crashed).toBe(1);
});

test('precision and recall count only exact diagnostics, and near misses are tallied by field', () => {
  const surge = okScore({ exact: 3, fp: 1, fn: 1, mismatched: 1, fields: { 'span-end': 1 }, fpKeys: ['a.ts:2:TS2322'], fnKeys: ['a.ts:2:TS2322'] });
  const results: TargetResult[] = [
    { name: 'a', tier: 'fixture', tsconfig: 'a', baselineStatus: 'ok', baseline: 4, scores: { 'surge-ts': surge, 'bolt-ts': okScore({ exact: 0, fp: 0, fn: 4 }) } },
  ];
  const agg = aggregate(results, 'surge-ts', true);
  expect(agg.precision).toBe(0.75);
  expect(agg.recall).toBe(0.75);
  expect(agg.exactTargets).toBe(0);
  expect(agg.fields).toStrictEqual({ 'span-end': 1 });
  const markdown = renderMarkdown(results, { date: 'd' });
  expect(markdown).toContain('| span-end | 1 | 0 | span end differs |');
  expect(markdown).not.toContain('file:line:code)');
});

test('maps one-to-one bolt-ts rewordings', () => {
  const templates = loadMessageTemplates();
  expect(codeForMessage("Property 'b' is missing.", templates)).toBe('TS2741');
  expect(codeForMessage("Generic type 'Box' requires 1 type argument.", templates)).toBe('TS2314');
});

test('renders a chart with a failure pill for a checker that crashed', () => {
  const results: TargetResult[] = [
    { name: 'a', tier: 'fixture', tsconfig: 'a', baselineStatus: 'ok', baseline: 1, scores: { 'surge-ts': okScore(), 'bolt-ts': okScore() } },
    { name: 'b', tier: 'fixture', tsconfig: 'b', baselineStatus: 'ok', baseline: 1, scores: { 'surge-ts': okScore(), 'bolt-ts': okScore({ status: 'crash' }) } },
  ];
  const svg = renderCheckerSvg(results, { date: '2026-09-18T00:00:00.000Z', tsgo: '7.0.2', platform: 'test' });
  expect(svg.startsWith('<svg')).toBeTruthy();
  expect(svg).toContain('1 crash');
  expect(svg).toContain('1 targets every checker completed');
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
