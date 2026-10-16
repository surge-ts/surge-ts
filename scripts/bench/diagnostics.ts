// Full-diagnostic comparison for bench:checkers: reads each tool's output with
// everything it shows (category, column, span, message chain) and scores a
// reported diagnostic as exact only when it equals a tsgo diagnostic in all of
// them.

import { readFileSync, realpathSync } from 'node:fs';
import path from 'node:path';

import { normalizeDiagnosticFileName, type NormalizedDiagnostic } from '../oracle/compare-tsc.js';

export type DiagnosticCategory = 'error' | 'warning' | 'suggestion' | 'message';

// 1-based line and 1-based UTF-16 character, the way tsc prints a position.
export type Position = { line: number; character: number };
export type TextRange = { start: Position; end: Position };

export type FullDiagnostic = NormalizedDiagnostic & {
  // Absolute path of the file, for reading its text; '' for global diagnostics.
  path?: string;
  // Undefined when the tool's output does not show it.
  category?: DiagnosticCategory;
  span?: TextRange;
  spanBytes?: { start: number; end: number };
  // Why `span` is missing, when the tool printed a location but no usable span.
  spanNote?: string;
  related?: Array<{ fileName: string; span?: TextRange; message: string }>;
};

export function realPath(file: string): string {
  try {
    return realpathSync.native(file);
  } catch {
    return path.resolve(file);
  }
}

function projectFileName(projectDir: string, absolute: string): string {
  return normalizeDiagnosticFileName(projectDir, absolute);
}

const categories: Record<string, DiagnosticCategory> = { error: 'error', warning: 'warning', suggestion: 'suggestion', message: 'message' };

// `--pretty false` output: `file(line,col): error TSxxxx: head`, then the
// message chain as lines indented two spaces per level. File names are
// relative to the tool's working directory.
export function parseTscStyleOutput(output: string, projectDir: string, cwd: string): FullDiagnostic[] {
  const diagnostics: FullDiagnostic[] = [];
  let current: FullDiagnostic | null = null;
  for (const raw of output.split('\n')) {
    const line = raw.replace(/\r$/, '');
    const located = /^(.+)\((\d+),(\d+)\): (error|warning|suggestion|message) (TS\d+): (.*)$/.exec(line);
    const global = located ? null : /^(error|warning|suggestion|message) (TS\d+): (.*)$/.exec(line);
    if (located) {
      const absolute = realPath(path.resolve(cwd, located[1]));
      current = {
        source: 'typescript',
        fileName: projectFileName(projectDir, absolute),
        path: absolute,
        line: Number(located[2]),
        column: Number(located[3]),
        category: categories[located[4]],
        code: located[5],
        message: located[6],
      };
      diagnostics.push(current);
    } else if (global) {
      current = { source: 'typescript', fileName: '', path: '', category: categories[global[1]], code: global[2], message: global[3] };
      diagnostics.push(current);
    } else if (current && line.startsWith('  ')) {
      current.message += `\n${line}`;
    } else {
      current = null;
    }
  }
  return diagnostics;
}

// A file's text as tsgo sees it: BOM removed, positions in UTF-16 units over
// ECMAScript line terminators. Byte offsets (surge-ts spans, bolt-ts columns)
// count UTF-8 bytes after the BOM.
export class SourceText {
  readonly text: string;
  private readonly bytes: Buffer | null;
  private readonly lineStarts: number[];
  private byteOffsets: Int32Array | null = null;
  private newlineStarts: number[] | null = null;

  constructor(raw: Buffer) {
    if (raw[0] === 0xff && raw[1] === 0xfe) {
      this.bytes = null;
      this.text = raw.subarray(2).toString('utf16le');
    } else {
      this.bytes = raw[0] === 0xef && raw[1] === 0xbb && raw[2] === 0xbf ? raw.subarray(3) : raw;
      this.text = this.bytes.toString('utf8');
    }
    this.lineStarts = ecmaLineStarts(this.text);
  }

  static read(file: string): SourceText | null {
    try {
      return new SourceText(readFileSync(file));
    } catch {
      return null;
    }
  }

  position(offset: number): Position {
    let low = 0;
    let high = this.lineStarts.length - 1;
    while (low < high) {
      const mid = (low + high + 1) >> 1;
      if (this.lineStarts[mid] <= offset) low = mid;
      else high = mid - 1;
    }
    return { line: low + 1, character: offset - this.lineStarts[low] + 1 };
  }

  // The UTF-16 offset of a UTF-8 byte offset, or null when it falls inside a
  // character or past the end. Invalid bytes decode to one U+FFFD each.
  offsetOfByte(byteOffset: number): number | null {
    const bytes = this.bytes;
    if (!bytes || byteOffset < 0 || byteOffset > bytes.length) return null;
    if (!this.byteOffsets) {
      const offsets = new Int32Array(bytes.length + 1).fill(-1);
      let utf16 = 0;
      let index = 0;
      while (index < bytes.length) {
        offsets[index] = utf16;
        const length = utf8SequenceLength(bytes, index);
        utf16 += length === 4 ? 2 : 1;
        index += length;
      }
      offsets[bytes.length] = utf16;
      this.byteOffsets = offsets;
    }
    const offset = this.byteOffsets[byteOffset];
    return offset < 0 ? null : offset;
  }

  positionOfByte(byteOffset: number): Position | null {
    const offset = this.offsetOfByte(byteOffset);
    return offset === null ? null : this.position(offset);
  }

  // miette numbers lines by `\n` alone, unlike tsgo's ECMAScript lines.
  mietteLineStartByte(line: number): number | null {
    const bytes = this.bytes;
    if (!bytes) return null;
    if (!this.newlineStarts) {
      const starts = [0];
      for (let index = 0; index < bytes.length; index++) {
        if (bytes[index] === 0x0a) starts.push(index + 1);
      }
      this.newlineStarts = starts;
    }
    return this.newlineStarts[line - 1] ?? null;
  }

  rawBytes(): Buffer | null {
    return this.bytes;
  }
}

function utf8SequenceLength(bytes: Buffer, index: number): number {
  const lead = bytes[index];
  const length = lead < 0x80 ? 1 : lead >= 0xf0 && lead < 0xf5 ? 4 : lead >= 0xe0 ? 3 : lead >= 0xc2 && lead < 0xe0 ? 2 : 1;
  if (length === 1) return 1;
  for (let next = 1; next < length; next++) {
    const byte = bytes[index + next];
    if (byte === undefined || (byte & 0xc0) !== 0x80) return 1;
  }
  return length;
}

function ecmaLineStarts(text: string): number[] {
  const starts = [0];
  for (let index = 0; index < text.length; index++) {
    const code = text.charCodeAt(index);
    if (code === 0x0d) {
      if (text.charCodeAt(index + 1) === 0x0a) index++;
      starts.push(index + 1);
    } else if (code === 0x0a || code === 0x2028 || code === 0x2029) {
      starts.push(index + 1);
    }
  }
  return starts;
}

function rangeFromOffsets(source: SourceText, start: number, end: number): TextRange {
  return { start: source.position(start), end: source.position(end) };
}

export function rangeFromBytes(source: SourceText, start: number, end: number): TextRange | null {
  const startPosition = source.positionOfByte(start);
  const endPosition = source.positionOfByte(end);
  return startPosition && endPosition ? { start: startPosition, end: endPosition } : null;
}

export function formatRange(range: TextRange): string {
  return `${range.start.line}:${range.start.character}-${range.end.line}:${range.end.character}`;
}

// What tsgo-api-diagnostics.mjs prints for one diagnostic: pos/end are UTF-16
// offsets into the BOM-less text.
export type ApiDiagnostic = {
  fileName?: string;
  pos: number;
  end: number;
  code: number;
  category: number;
  text: string;
  messageChain?: ApiDiagnostic[];
  relatedInformation?: ApiDiagnostic[];
};

// The message as `--pretty false` prints it, so CLI and API diagnostics can be
// paired on their text.
export function flattenApiMessage(diagnostic: ApiDiagnostic): string {
  let text = diagnostic.text;
  const visit = (chain: ApiDiagnostic, level: number) => {
    text += `\n${'  '.repeat(level)}${chain.text}`;
    for (const next of chain.messageChain ?? []) visit(next, level + 1);
  };
  for (const next of diagnostic.messageChain ?? []) visit(next, 1);
  return text;
}

// The CLI decides which diagnostics tsgo reports (it skips semantic checking
// after syntax or option errors; the API does not), so the CLI's set is the
// baseline and the API only lends each of them its span and related
// information. Pairs agree on file, code and start position, preferring the
// same message.
export function attachApiSpans(
  cli: FullDiagnostic[],
  api: ApiDiagnostic[],
  projectDir: string,
  readSource: (file: string) => SourceText | null,
): void {
  const candidates = new Map<string, Array<{ diagnostic: ApiDiagnostic; path: string; message: string; used: boolean }>>();
  for (const diagnostic of api) {
    if (!diagnostic.fileName) continue;
    const file = realPath(diagnostic.fileName);
    const key = `${file}\0TS${diagnostic.code}`;
    const list = candidates.get(key) ?? [];
    list.push({ diagnostic, path: file, message: flattenApiMessage(diagnostic), used: false });
    candidates.set(key, list);
  }
  for (const diagnostic of cli) {
    if (!diagnostic.path) continue;
    const source = readSource(diagnostic.path);
    if (!source) continue;
    const toRange = (entry: ApiDiagnostic) => rangeFromOffsets(source, entry.pos, entry.end);
    const atStart = (candidates.get(`${diagnostic.path}\0${diagnostic.code}`) ?? []).filter((candidate) => {
      if (candidate.used) return false;
      const start = source.position(candidate.diagnostic.pos);
      return start.line === diagnostic.line && start.character === diagnostic.column;
    });
    const match = atStart.find((candidate) => candidate.message === diagnostic.message) ?? atStart[0];
    if (!match) continue;
    match.used = true;
    diagnostic.span = toRange(match.diagnostic);
    if (match.diagnostic.relatedInformation?.length) {
      diagnostic.related = match.diagnostic.relatedInformation.map((related) => {
        const relatedPath = related.fileName ? realPath(related.fileName) : '';
        const relatedSource = relatedPath ? readSource(relatedPath) : null;
        return {
          fileName: relatedPath ? projectFileName(projectDir, relatedPath) : '',
          span: relatedSource ? rangeFromOffsets(relatedSource, related.pos, related.end) : undefined,
          message: flattenApiMessage(related),
        };
      });
    }
  }
}

// `surge --format json`: file names relative to the project directory, spans
// as UTF-8 byte offsets; no category.
export function parseSurgeJson(output: string, projectDir: string): FullDiagnostic[] {
  const parsed = JSON.parse(output) as { diagnostics?: unknown };
  const entries = Array.isArray(parsed.diagnostics) ? parsed.diagnostics : [];
  return entries.map((entry) => {
    const item = entry as { code?: unknown; fileName?: unknown; line?: unknown; column?: unknown; message?: unknown; span?: { start?: unknown; end?: unknown } };
    const raw = typeof item.fileName === 'string' ? item.fileName : '';
    const absolute = raw ? realPath(path.resolve(projectDir, raw)) : '';
    const span = item.span;
    return {
      source: 'surge-ts' as const,
      code: String(item.code ?? ''),
      fileName: absolute ? projectFileName(projectDir, absolute) : '',
      path: absolute,
      line: typeof item.line === 'number' ? item.line : undefined,
      column: typeof item.column === 'number' ? item.column : undefined,
      message: typeof item.message === 'string' ? item.message : '',
      spanBytes: span && typeof span.start === 'number' && typeof span.end === 'number' ? { start: span.start, end: span.end } : undefined,
    };
  });
}

// Lends each JSON diagnostic the category its `--diagnosticStyle tsc` twin
// shows. The text output groups by file, so twins pair on content, not order.
export function mergeSurgeOutputs(json: FullDiagnostic[], text: FullDiagnostic[] | null): FullDiagnostic[] {
  if (!text) return json;
  const used = new Set<FullDiagnostic>();
  return json.map((diagnostic) => {
    const twin = text.find((candidate) =>
      !used.has(candidate) &&
      candidate.fileName === diagnostic.fileName &&
      candidate.line === diagnostic.line &&
      candidate.code === diagnostic.code &&
      headOf(candidate.message ?? '') === headOf(diagnostic.message ?? ''),
    );
    if (!twin) return diagnostic;
    used.add(twin);
    return { ...diagnostic, category: twin.category };
  });
}

export function attachByteSpans(diagnostics: FullDiagnostic[], readSource: (file: string) => SourceText | null): FullDiagnostic[] {
  return diagnostics.map((diagnostic) => {
    if (!diagnostic.path) return diagnostic;
    if (!diagnostic.spanBytes) return { ...diagnostic, spanNote: 'no span in the output' };
    const source = readSource(diagnostic.path);
    if (!source) return { ...diagnostic, spanNote: 'file unreadable' };
    const span = rangeFromBytes(source, diagnostic.spanBytes.start, diagnostic.spanBytes.end);
    return span ? { ...diagnostic, span } : { ...diagnostic, spanNote: 'byte offset not on a character boundary' };
  });
}

export type BoltSnippetRow = { line: number; text: string; marks?: string };

export type BoltDiagnostic = {
  message: string;
  fileName: string;
  line?: number;
  column?: number;
  category: DiagnosticCategory;
  snippet: BoltSnippetRow[];
  multiLineLabel?: boolean;
};

// Parses miette's unicode no-color graphical report. Top-level diagnostics
// start with `  × ` (error) or `  ⚠ ` (warning); miette wraps long messages at
// 80 columns with `  │ ` continuation lines. `Advice:` blocks (`☞`) are
// related information attached to the previous diagnostic and are skipped.
// bolt-ts hands miette only the source its labels cover, so the first snippet
// row starts at the first label rather than at the line start; the location
// header names the primary label (a byte column).
export function parseBoltOutput(output: string): BoltDiagnostic[] {
  const diagnostics: BoltDiagnostic[] = [];
  let current: BoltDiagnostic | null = null;
  let inMessage = false;
  let inAdvice = false;
  let inSnippet = false;
  for (const line of output.split(/\r?\n/)) {
    const head = /^  ([×⚠]) (.*)$/.exec(line);
    if (head) {
      current = { message: head[2].trim(), fileName: '', category: head[1] === '×' ? 'error' : 'warning', snippet: [] };
      diagnostics.push(current);
      inMessage = true;
      inAdvice = false;
      inSnippet = false;
      continue;
    }
    if (/^  ☞ /.test(line) || line.startsWith('Advice:')) {
      inAdvice = true;
      inMessage = false;
      inSnippet = false;
      continue;
    }
    if (!current || inAdvice) {
      continue;
    }
    const continuation = /^  │ (.*)$/.exec(line);
    if (inMessage && continuation) {
      current.message += ` ${continuation[1].trim()}`;
      continue;
    }
    inMessage = false;
    const location = /╭─\[(.+):(\d+):(\d+)\]/.exec(line);
    if (location) {
      inSnippet = !current.fileName;
      if (inSnippet) {
        current.fileName = location[1];
        current.line = Number(location[2]);
        current.column = Number(location[3]);
      }
      continue;
    }
    if (!inSnippet) continue;
    if (/^\s*╰─/.test(line)) {
      inSnippet = false;
      continue;
    }
    const row = /^\s*(\d+) │ ?(.*)$/.exec(line);
    if (row) {
      const text = row[2];
      if (current.snippet.length === 0 && text.startsWith('╭─▶')) current.multiLineLabel = true;
      current.snippet.push({ line: Number(row[1]), text });
      continue;
    }
    const marks = /^\s*· ?(.*)$/.exec(line);
    const last = current.snippet.at(-1);
    if (marks && last && last.marks === undefined) last.marks = marks[1];
  }
  return diagnostics;
}

// The span the underline shows, as UTF-8 byte offsets into the file. The
// underline is measured in display columns, so the source characters it covers
// are found by laying the source line out the way miette does. A row may also
// underline secondary labels (a duplicate's first declaration); the primary
// label is the underline run that begins at the header's column.
export function recoverBoltSpan(diagnostic: BoltDiagnostic, source: SourceText): { start: number; end: number } | { unobservable: string } {
  if (diagnostic.line === undefined || diagnostic.column === undefined) return { unobservable: 'no location' };
  if (diagnostic.multiLineLabel) return { unobservable: 'multi-line label: end column not printed' };
  const row = diagnostic.snippet.find((candidate) => candidate.line === diagnostic.line);
  const marks = row?.marks?.trimEnd() ?? '';
  if (!row || !/[─┬┴^▲]/.test(marks)) return { unobservable: 'no underline under the primary line' };
  const lineStart = source.mietteLineStartByte(diagnostic.line);
  const bytes = source.rawBytes();
  if (lineStart === null || !bytes) return { unobservable: 'line outside the file' };
  const start = lineStart + diagnostic.column - 1;
  const lineEnd = bytes.indexOf(0x0a, lineStart);
  const line = [...bytes.subarray(lineStart, lineEnd < 0 ? bytes.length : lineEnd).toString('utf8').replace(/\r$/, '')];
  const rowText = row.text.trimEnd();
  // Only the first row can start inside its line; take the latest start at or
  // before the label whose layout reproduces the row.
  let rowFrom: number | null = null;
  let byte = 0;
  for (let index = 0; index <= line.length && lineStart + byte <= start; index++) {
    if ((row === diagnostic.snippet[0] || index === 0) && renderMietteText(line.slice(index)).startsWith(rowText)) rowFrom = index;
    if (index < line.length) byte += Buffer.byteLength(line[index], 'utf8');
  }
  if (rowFrom === null) return { unobservable: 'snippet text differs from the source' };
  const chars = line.slice(rowFrom);
  const widths = mietteWidths(chars);
  let labelChar = 0;
  let offset = 0;
  while (labelChar < chars.length && lineStart + byteLength(line, 0, rowFrom + labelChar) < start) offset += widths[labelChar++];
  if (lineStart + byteLength(line, 0, rowFrom + labelChar) !== start) return { unobservable: 'label column inside a character' };
  const run = /^[─┬┴^▲]+/.exec([...marks].slice(offset).join(''));
  if (!run || (offset > 0 && /[─┬┴^▲]/.test([...marks][offset - 1]))) return { unobservable: 'no underline under the primary label' };
  const width = [...run[0]].length;
  let column = 0;
  let to: number | null = null;
  for (let index = labelChar; index <= chars.length; index++) {
    if (column === width) {
      to = index;
      break;
    }
    if (index === chars.length) break;
    column += widths[index];
  }
  if (to === null) return { unobservable: 'span runs past the end of the line' };
  return { start, end: start + byteLength(chars, labelChar, to) };
}

function byteLength(chars: string[], from: number, to: number): number {
  return Buffer.byteLength(chars.slice(from, to).join(''), 'utf8');
}

const mietteTabWidth = 4;

// Display width of each character as miette lays out a row: a tab runs to the
// next multiple of four columns.
function mietteWidths(chars: string[]): number[] {
  let column = 0;
  return chars.map((char) => {
    const width = char === '\t' ? mietteTabWidth - (column % mietteTabWidth) : charWidth(char.codePointAt(0) ?? 0);
    column += width;
    return width;
  });
}

function renderMietteText(chars: string[]): string {
  const widths = mietteWidths(chars);
  return chars.map((char, index) => (char === '\t' ? ' '.repeat(widths[index]) : char)).join('');
}

// An approximation of unicode-width: combining marks and zero-width characters
// take no column, East Asian wide characters and emoji take two.
function charWidth(codePoint: number): number {
  const char = String.fromCodePoint(codePoint);
  if (/\p{Mn}|\p{Me}|[​-‏⁠﻿]/u.test(char)) return 0;
  if (
    /\p{Emoji_Presentation}/u.test(char) ||
    (codePoint >= 0x1100 && codePoint <= 0x115f) ||
    (codePoint >= 0x2e80 && codePoint <= 0xa4cf && codePoint !== 0x303f) ||
    (codePoint >= 0xac00 && codePoint <= 0xd7a3) ||
    (codePoint >= 0xf900 && codePoint <= 0xfaff) ||
    (codePoint >= 0xfe30 && codePoint <= 0xfe4f) ||
    (codePoint >= 0xff00 && codePoint <= 0xff60) ||
    (codePoint >= 0xffe0 && codePoint <= 0xffe6) ||
    (codePoint >= 0x20000 && codePoint <= 0x3fffd)
  ) {
    return 2;
  }
  return 1;
}

export type MismatchField =
  | 'column'
  | 'category'
  | 'category-unobservable'
  | 'message-text'
  | 'message-chain-missing'
  | 'message-chain-extra'
  | 'message-chain-differs'
  | 'span-start'
  | 'span-end'
  | 'span-unobservable';

export const mismatchMeanings: Record<MismatchField, string> = {
  column: 'column as printed differs (tsgo counts UTF-16 units)',
  category: 'category differs',
  'category-unobservable': "the tool's output does not show the category",
  'message-text': 'first line of the message differs',
  'message-chain-missing': 'tsgo elaborates with a message chain the tool omits',
  'message-chain-extra': 'the tool elaborates where tsgo does not',
  'message-chain-differs': 'both elaborate, differently',
  'span-start': 'span start differs',
  'span-end': 'span end differs',
  'span-unobservable': "the tool's output gives no recoverable span",
};

// `exact` compares messages character for character; `whitespace` collapses
// whitespace runs first, for output that rewraps messages (miette).
export type MessageMode = 'exact' | 'whitespace';

const collapse = (text: string) => text.replace(/\s+/g, ' ').trim();
const headOf = (text: string) => text.split('\n', 1)[0];
const chainOf = (text: string) => text.slice(headOf(text).length);
const samePosition = (a: Position, b: Position) => a.line === b.line && a.character === b.character;

export function compareDiagnostic(expected: FullDiagnostic, actual: FullDiagnostic, mode: MessageMode): MismatchField[] {
  const fields: MismatchField[] = [];
  if (expected.column !== actual.column) fields.push('column');
  if (actual.category === undefined) fields.push('category-unobservable');
  else if (expected.category !== actual.category) fields.push('category');
  const norm = mode === 'whitespace' ? collapse : (text: string) => text;
  const expectedMessage = expected.message ?? '';
  const actualMessage = actual.message ?? '';
  if (norm(expectedMessage) !== norm(actualMessage)) {
    if (norm(headOf(expectedMessage)) !== norm(headOf(actualMessage))) fields.push('message-text');
    const expectedChain = chainOf(expectedMessage);
    const actualChain = chainOf(actualMessage);
    if (expectedChain && !actualChain) fields.push('message-chain-missing');
    else if (!expectedChain && actualChain) fields.push('message-chain-extra');
    else if (norm(expectedChain) !== norm(actualChain)) fields.push('message-chain-differs');
  }
  if (expected.span) {
    if (!actual.span) fields.push('span-unobservable');
    else {
      if (!samePosition(expected.span.start, actual.span.start)) fields.push('span-start');
      if (!samePosition(expected.span.end, actual.span.end)) fields.push('span-end');
    }
  }
  return fields;
}

export type DiagnosticSnapshot = {
  file: string;
  line?: number;
  column?: number;
  code: string;
  category?: string;
  span?: string;
  message: string;
};

export type MismatchExample = { fields: MismatchField[]; expected: DiagnosticSnapshot; actual: DiagnosticSnapshot };

export type StrictScore = {
  // Reported diagnostics equal to a tsgo diagnostic in every compared field.
  exact: number;
  fp: number;
  fn: number;
  // Near misses: pairs that share file, line and code but differ elsewhere.
  // Each is one FP and one FN.
  mismatched: number;
  fields: Partial<Record<MismatchField, number>>;
  // Located pairs compared without a span because tsgo's span was not available.
  spanless: number;
  // `file:line:code` of every reported / expected diagnostic that is not exact.
  fpKeys: string[];
  fnKeys: string[];
  examples: MismatchExample[];
};

function snapshot(diagnostic: FullDiagnostic): DiagnosticSnapshot {
  return {
    file: diagnostic.fileName,
    line: diagnostic.line,
    column: diagnostic.column,
    code: diagnostic.code,
    category: diagnostic.category,
    span: diagnostic.span ? formatRange(diagnostic.span) : undefined,
    message: diagnostic.message ?? '',
  };
}

// The code stays last: report code tallies read it after the final `:`.
export const lineKey = (diagnostic: NormalizedDiagnostic) => `${diagnostic.fileName}:${diagnostic.line ?? 0}:${diagnostic.code}`;

// Within each file:line:code group, exact pairs are taken first; what remains
// is paired as near misses, each reported diagnostic with the expected one it
// differs from in the fewest fields.
export function scoreStrict(
  baseline: FullDiagnostic[],
  reported: FullDiagnostic[],
  mode: MessageMode,
  exampleLimit = 3,
): StrictScore {
  const group = (list: FullDiagnostic[]) => {
    const groups = new Map<string, FullDiagnostic[]>();
    for (const diagnostic of list) {
      const key = lineKey(diagnostic);
      const members = groups.get(key);
      if (members) members.push(diagnostic);
      else groups.set(key, [diagnostic]);
    }
    return groups;
  };
  const expectedGroups = group(baseline);
  const actualGroups = group(reported);
  const score: StrictScore = { exact: 0, fp: 0, fn: 0, mismatched: 0, fields: {}, spanless: 0, fpKeys: [], fnKeys: [], examples: [] };
  const spanless = (expected: FullDiagnostic) => (expected.fileName && !expected.span ? 1 : 0);
  for (const key of new Set([...expectedGroups.keys(), ...actualGroups.keys()])) {
    const expected = [...(expectedGroups.get(key) ?? [])];
    const actual = [...(actualGroups.get(key) ?? [])];
    for (let index = actual.length - 1; index >= 0; index--) {
      const twin = expected.findIndex((candidate) => compareDiagnostic(candidate, actual[index], mode).length === 0);
      if (twin < 0) continue;
      score.exact++;
      score.spanless += spanless(expected[twin]);
      expected.splice(twin, 1);
      actual.splice(index, 1);
    }
    score.fpKeys.push(...actual.map(() => key));
    score.fnKeys.push(...expected.map(() => key));
    for (const diagnostic of actual) {
      let best = -1;
      let bestFields: MismatchField[] = [];
      expected.forEach((candidate, index) => {
        const fields = compareDiagnostic(candidate, diagnostic, mode);
        if (best < 0 || fields.length < bestFields.length) {
          best = index;
          bestFields = fields;
        }
      });
      if (best < 0) break;
      score.mismatched++;
      score.spanless += spanless(expected[best]);
      for (const field of bestFields) score.fields[field] = (score.fields[field] ?? 0) + 1;
      if (score.examples.length < exampleLimit) {
        score.examples.push({ fields: bestFields, expected: snapshot(expected[best]), actual: snapshot(diagnostic) });
      }
      expected.splice(best, 1);
    }
  }
  score.fp = reported.length - score.exact;
  score.fn = baseline.length - score.exact;
  return score;
}
