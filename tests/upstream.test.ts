import { readFileSync } from 'node:fs';
import { relative, sep } from 'node:path';

import { expect, test } from 'vitest';

import { WORKSPACE_ROOT } from './harness/paths.ts';
import { fixturePaths, fixtureSuite, workspacePath } from './harness/suite.ts';

const CASES = workspacePath('tests/upstream/typescript-go/cases');

fixtureSuite(CASES);

test('every vendored upstream case records its provenance', () => {
	const manifest = readFileSync(
		workspacePath('tests/upstream/typescript-go/manifest.toml'),
		'utf8',
	);
	const recorded = new Set(
		[...manifest.matchAll(/^local_path = "([^"]+)"$/gm)].map((match) => match[1]),
	);
	const vendored = fixturePaths(CASES).map((path) =>
		relative(WORKSPACE_ROOT, path).split(sep).join('/'),
	);
	expect(vendored.filter((path) => !recorded.has(path))).toEqual([]);
	expect([...recorded].filter((path) => !vendored.includes(path))).toEqual([]);
});
