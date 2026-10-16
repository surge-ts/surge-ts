import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { afterAll, expect, test } from 'vitest';

import {
  attachApiSpans,
  attachByteSpans,
  compareDiagnostic,
  flattenApiMessage,
  mergeSurgeOutputs,
  parseBoltOutput,
  parseSurgeJson,
  parseTscStyleOutput,
  realPath,
  recoverBoltSpan,
  scoreStrict,
  SourceText,
  type ApiDiagnostic,
  type FullDiagnostic,
} from './diagnostics.js';

const dirs: string[] = [];
const tempDir = () => {
  const dir = realPath(mkdtempSync(path.join(os.tmpdir(), 'bench-diagnostics-')));
  dirs.push(dir);
  return dir;
};
afterAll(() => {
  for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
});

const bom = Buffer.from([0xef, 0xbb, 0xbf]);

test('reads tsc-style output with message chains and global diagnostics', () => {
  const dir = tempDir();
  const output = [
    "src/a.ts(6,5): error TS2322: Type 'A' is not assignable to type 'B'.",
    "  Type 'string' is not assignable to type 'number'.",
    '    Deeper.',
    "error TS5023: Unknown compiler option 'x'.",
    'Found 2 errors.',
  ].join('\n');
  const diagnostics = parseTscStyleOutput(output, dir, dir);
  expect(diagnostics).toHaveLength(2);
  expect(diagnostics[0]).toMatchObject({
    fileName: 'src/a.ts',
    path: path.join(dir, 'src', 'a.ts'),
    line: 6,
    column: 5,
    category: 'error',
    code: 'TS2322',
    message: "Type 'A' is not assignable to type 'B'.\n  Type 'string' is not assignable to type 'number'.\n    Deeper.",
  });
  expect(diagnostics[1]).toMatchObject({ fileName: '', code: 'TS5023', category: 'error' });
});

test('source positions are UTF-16 over ECMAScript lines, byte offsets skip the BOM', () => {
  const source = new SourceText(Buffer.concat([bom, Buffer.from('a🙂b\r\nc d\n한x')]));
  expect(source.position(0)).toStrictEqual({ line: 1, character: 1 });
  expect(source.position(3)).toStrictEqual({ line: 1, character: 4 });
  expect(source.position(6)).toStrictEqual({ line: 2, character: 1 });
  expect(source.position(8)).toStrictEqual({ line: 3, character: 1 });
  expect(source.offsetOfByte(1)).toBe(1);
  expect(source.offsetOfByte(2)).toBe(null);
  expect(source.offsetOfByte(5)).toBe(3);
  // Line 4 for tsgo (U+2028 ends a line), line 3 for miette.
  const hangul = Buffer.byteLength('a🙂b\r\nc d\n');
  expect(source.mietteLineStartByte(3)).toBe(hangul);
  expect(source.positionOfByte(hangul + 3)).toStrictEqual({ line: 4, character: 2 });
});

test('flattens API message chains the way the CLI prints them', () => {
  const chain = (text: string, messageChain?: ApiDiagnostic[]): ApiDiagnostic => ({ pos: 0, end: 0, code: 1, category: 1, text, messageChain });
  expect(flattenApiMessage(chain('head', [chain('one', [chain('two')]), chain('sibling')]))).toBe('head\n  one\n    two\n  sibling');
});

test('lends CLI diagnostics the span and related information of their API twin', () => {
  const dir = tempDir();
  const file = path.join(dir, 'a.ts');
  writeFileSync(file, Buffer.concat([bom, Buffer.from('let s = "🙂"; let q: number = s;\nf(1);\n')]));
  const cli = parseTscStyleOutput(
    ["a.ts(1,19): error TS2322: Type 'string' is not assignable to type 'number'.", 'a.ts(2,1): error TS2554: Expected 2 arguments, but got 1.'].join('\n'),
    dir,
    dir,
  );
  const api: ApiDiagnostic[] = [
    { fileName: file, pos: 18, end: 19, code: 2322, category: 1, text: "Type 'string' is not assignable to type 'number'." },
    {
      fileName: file, pos: 33, end: 34, code: 2554, category: 1, text: 'Expected 2 arguments, but got 1.',
      relatedInformation: [{ fileName: file, pos: 0, end: 3, code: 6210, category: 3, text: "An argument for 'b' was not provided." }],
    },
  ];
  attachApiSpans(cli, api, dir, SourceText.read);
  expect(cli[0].span).toStrictEqual({ start: { line: 1, character: 19 }, end: { line: 1, character: 20 } });
  expect(cli[1].span).toStrictEqual({ start: { line: 2, character: 1 }, end: { line: 2, character: 2 } });
  expect(cli[1].related).toStrictEqual([
    { fileName: 'a.ts', span: { start: { line: 1, character: 1 }, end: { line: 1, character: 4 } }, message: "An argument for 'b' was not provided." },
  ]);
});

test('surge-ts: categories come from the tsc-style twin, spans from JSON byte offsets', () => {
  const dir = tempDir();
  writeFileSync(path.join(dir, 'a.ts'), Buffer.concat([bom, Buffer.from('const k = "한"; let q: number = k;\n')]));
  const json = parseSurgeJson(
    JSON.stringify({
      diagnostics: [
        { code: 'TS2322', fileName: 'a.ts', message: "Type 'string' is not assignable to type 'number'.", span: { start: 21, end: 22 }, line: 1, column: 20 },
        { code: 'TS2304', fileName: 'a.ts', message: "Cannot find name 'x'.", span: { start: 11, end: 12 }, line: 1, column: 12 },
      ],
    }),
    dir,
  );
  const text = parseTscStyleOutput("a.ts(1,20): error TS2322: Type 'string' is not assignable to type 'number'.", dir, dir);
  const merged = attachByteSpans(mergeSurgeOutputs(json, text), SourceText.read);
  expect(merged[0].category).toBe('error');
  expect(merged[0].span).toStrictEqual({ start: { line: 1, character: 20 }, end: { line: 1, character: 21 } });
  expect(merged[1].category).toBe(undefined);
  expect(merged[1].spanNote).toBe('byte offset not on a character boundary');
});

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
  "  ⚠ Cannot find name 'Iterator'.",
  "  [Failed to read contents for label (offset: 71948, length: 7): OutOfBounds]",
  '',
  '  × Type is not assignable.',
  '   ╭─[c.ts:2:11]',
  ' 2 │ ╭─▶ function* () {',
  ' 3 │ │           yield x => x.length;',
  ' 4 │ ╰─▶     } ()',
  '   ╰────',
  '',
  'Files: 58',
  'Types: 287',
  'Time cost: 17ms',
].join('\n');

test('parses miette reports: wrapped messages, category, snippet rows; advice skipped', () => {
  const parsed = parseBoltOutput(boltOutput);
  expect(parsed.map(({ message, fileName, line, column, category }) => ({ message, fileName, line, column, category }))).toStrictEqual([
    { message: "Type 'number[]' is not assignable to type 'Iterable<string, any, any>'.", fileName: 'src/index.ts', line: 15, column: 14, category: 'error' },
    {
      message: "Non-abstract class 'MissingGeneric' does not implement inherited abstract member 'handle' from class 'Generic<string>'.",
      fileName: 'src/index.ts', line: 54, column: 8, category: 'error',
    },
    { message: "Cannot find name 'Iterator'.", fileName: '', line: undefined, column: undefined, category: 'warning' },
    { message: 'Type is not assignable.', fileName: 'c.ts', line: 2, column: 11, category: 'error' },
  ]);
  expect(parsed[0].snippet).toStrictEqual([{ line: 15, text: 'wrongElement', marks: '────────────' }]);
  expect(parsed[1].snippet).toStrictEqual([{ line: 54, text: 'class MissingGeneric extends Generic<string> {}' }]);
  expect(parsed[3].multiLineLabel).toBe(true);
});

test('recovers bolt-ts spans from the underline across tabs and wide characters', () => {
  const source = new SourceText(Buffer.concat([bom, Buffer.from('function g(): Array<\tnumber> {}\nlet e = "한🙂"; x;\n')]));
  const at = (line: number, column: number, text: string, marks: string) =>
    recoverBoltSpan({ message: '', fileName: 'a.ts', line, column, category: 'error', snippet: [{ line, text, marks }] }, source);
  expect(at(1, 15, 'Array<  number>', '───────────────')).toStrictEqual({ start: 14, end: 28 });
  const quoted = Buffer.byteLength('let e = ');
  expect(at(2, quoted + 1, '"한🙂"', '──────')).toStrictEqual({ start: 32 + quoted, end: 32 + quoted + Buffer.byteLength('"한🙂"') });
  expect(at(1, 15, 'Array<  number>', '────────────────────────────')).toStrictEqual({ unobservable: 'span runs past the end of the line' });
  expect(at(1, 15, 'Other', '─────')).toStrictEqual({ unobservable: 'snippet text differs from the source' });
  expect(recoverBoltSpan({ message: '', fileName: 'a.ts', line: 1, column: 1, category: 'error', snippet: [{ line: 1, text: 'function' }] }, source))
    .toStrictEqual({ unobservable: 'no underline under the primary line' });
});

test('picks the primary label out of a row that also underlines a secondary one', () => {
  const source = new SourceText(Buffer.from('export enum Dup { X, X }\nclass C {\n    x = 1;\n    get x() {}\n}\n'));
  const primary = (line: number, column: number, snippet: Array<{ line: number; text: string; marks?: string }>) =>
    recoverBoltSpan({ message: '', fileName: 'a.ts', line, column, category: 'error', snippet }, source);
  expect(primary(1, 22, [{ line: 1, text: 'X, X', marks: '┬  ─' }])).toStrictEqual({ start: 21, end: 22 });
  const getter = Buffer.byteLength('export enum Dup { X, X }\nclass C {\n    x = 1;\n    get ');
  expect(primary(4, 9, [{ line: 3, text: 'x = 1;', marks: '┬' }, { line: 4, text: '    get x', marks: '        ─' }]))
    .toStrictEqual({ start: getter, end: getter + 1 });
});

const diag = (overrides: Partial<FullDiagnostic> = {}): FullDiagnostic => ({
  source: 'typescript',
  fileName: 'a.ts',
  line: 1,
  column: 5,
  code: 'TS2322',
  category: 'error',
  message: "Type 'string' is not assignable to type 'number'.",
  span: { start: { line: 1, character: 5 }, end: { line: 1, character: 6 } },
  ...overrides,
});

test('compares every field and names each one that differs', () => {
  expect(compareDiagnostic(diag(), diag(), 'exact')).toStrictEqual([]);
  expect(compareDiagnostic(diag({ message: 'head\n  chain' }), diag({ message: 'head' }), 'exact')).toStrictEqual(['message-chain-missing']);
  expect(compareDiagnostic(diag({ message: 'head' }), diag({ message: 'head\n  chain' }), 'exact')).toStrictEqual(['message-chain-extra']);
  expect(compareDiagnostic(diag(), diag({ column: 4, category: undefined, span: undefined, message: 'other' }), 'exact'))
    .toStrictEqual(['column', 'category-unobservable', 'message-text', 'span-unobservable']);
  expect(compareDiagnostic(diag(), diag({ span: { start: { line: 1, character: 5 }, end: { line: 1, character: 9 } } }), 'exact')).toStrictEqual(['span-end']);
  expect(compareDiagnostic(diag({ message: 'a  b' }), diag({ message: 'a b' }), 'exact')).toStrictEqual(['message-text']);
  expect(compareDiagnostic(diag({ message: 'a  b' }), diag({ message: 'a b' }), 'whitespace')).toStrictEqual([]);
  expect(compareDiagnostic(diag({ span: undefined }), diag({ span: undefined }), 'exact')).toStrictEqual([]);
});

test('scores exact pairs first, then near misses within each file:line:code group', () => {
  const baseline = [diag(), diag({ column: 9, span: undefined }), diag({ line: 3, code: 'TS2304', message: "Cannot find name 'x'." })];
  const reported = [diag({ column: 9 }), diag(), diag({ line: 3, code: 'TS2552', message: "Cannot find name 'x'. Did you mean 'y'?" }), diag({ fileName: 'b.ts' })];
  const score = scoreStrict(baseline, reported, 'exact');
  expect(score.exact).toBe(2);
  expect(score.fp).toBe(2);
  expect(score.fn).toBe(1);
  expect(score.mismatched).toBe(0);
  expect(score.spanless).toBe(1);
  expect(score.fpKeys.sort()).toStrictEqual(['a.ts:3:TS2552', 'b.ts:1:TS2322']);
  expect(score.fnKeys).toStrictEqual(['a.ts:3:TS2304']);

  const near = scoreStrict([diag()], [diag({ span: { start: { line: 1, character: 5 }, end: { line: 1, character: 7 } } })], 'exact');
  expect(near).toMatchObject({ exact: 0, fp: 1, fn: 1, mismatched: 1, fields: { 'span-end': 1 }, fpKeys: ['a.ts:1:TS2322'], fnKeys: ['a.ts:1:TS2322'] });
  expect(near.examples[0].expected.span).toBe('1:5-1:6');
});
