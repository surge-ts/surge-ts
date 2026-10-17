import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

import { expect, test } from 'vitest';

const scriptPath = fileURLToPath(import.meta.url);
const scriptDir = path.dirname(scriptPath);
const workspaceRoot = path.resolve(scriptDir, '../..');
const compareScript = path.join(scriptDir, 'compare-tsc.ts');

const bannedTerms = [
  'CategorizedCountEntry',
  'nodeModulesSourceDiagnostics',
  'nodeModulesJavaScriptSourceDiagnostics',
  'node_modules source',
  'dependency JavaScript',
  'source-prefix',
  'candidate',
  'synthetic built-in',
  'ES/lib-lite',
  'Node-like',
  'DOM-like',
  'JSX-like',
  'local-unresolved',
  'simplewebauthn',
  'uuid',
  'react',
  'noble',
];

test('oracle source stays raw and classifier-free', () => {
  const files = [
    path.join(scriptDir, 'compare-tsc.ts'),
    path.join(scriptDir, 'compare-tsc.test.ts'),
    path.join(scriptDir, 'README.md'),
  ];

  const source = files
    .map((file) => withoutPresetRegistryEntries(fs.readFileSync(file, 'utf8')))
    .join('\n');

  for (const term of bannedTerms) {
    expect(
      containsWholeTerm(source, term),
      `oracle source still contains banned term: ${term}`,
    ).toBe(false);
  }
});

test('oracle output stays raw on a tiny unresolved project', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'oracle-guard-'));
  const tsconfig = path.join(root, 'tsconfig.json');
  const entry = path.join(root, 'src', 'index.ts');

  fs.mkdirSync(path.dirname(entry), { recursive: true });
  fs.writeFileSync(
    tsconfig,
    JSON.stringify({ compilerOptions: { strict: true, noEmit: true }, include: ['src/**/*.ts'] }),
  );
  fs.writeFileSync(entry, 'import { MissingThing } from "./missing";\nexport const value: MissingThing = UnknownGlobal;\n');

  const result = spawnSync(
    process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm',
    ['exec', 'tsx', compareScript, '--project', tsconfig, '--maxDiagnostics', '200'],
    {
      cwd: workspaceRoot,
      encoding: 'utf8',
      shell: process.platform === 'win32',
    },
  );

  expect(result.status, result.stderr || result.stdout).toBe(0);
  const output = `${result.stdout ?? ''}\n${result.stderr ?? ''}`;

  for (const term of bannedTerms) {
    expect(
      containsWholeTerm(output, term),
      `oracle output still contains banned term: ${term}`,
    ).toBe(false);
  }
});

// Preset-registry entries name compat-project fixtures, not diagnostic
// classifiers: a fixture may legitimately be called e.g. `...-candidate-...`
// after the type-system concept it pins. Only lines matching the exact
// `'preset': path.join(workspaceRoot, '...')` shape are exempt, and when the
// registry cannot be located the whole file is scanned as before.
const presetRegistryStart = /^export const fixturePresets: Record<string, string> = \{$/;
const presetRegistryEntry = /^\s*'[^']+': path\.join\(workspaceRoot, '[^']+'\),$/;

function withoutPresetRegistryEntries(source: string): string {
  const lines = source.split('\n');
  const start = lines.findIndex((line) => presetRegistryStart.test(line));
  const end = start === -1 ? -1 : lines.indexOf('};', start);
  if (end === -1) {
    return source;
  }

  return lines
    .map((line, index) =>
      index > start && index < end && presetRegistryEntry.test(line) ? '' : line,
    )
    .join('\n');
}

function containsWholeTerm(source: string, term: string): boolean {
  const escaped = term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp(`(^|[^A-Za-z0-9_])${escaped}([^A-Za-z0-9_]|$)`, 'i');
  return pattern.test(source);
}
