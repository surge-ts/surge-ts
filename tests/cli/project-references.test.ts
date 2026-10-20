import { test } from 'vitest';

import { tempProject } from '../harness/cli.ts';
import { expectSameAsTsc } from './project-mode-helpers.ts';

// A referenced project's sources reach a program as that project's output
// declarations. The outputs these tests write disagree with the sources, so
// which one the program read shows in the diagnostics.

const COMPOSITE = { composite: true, declaration: true, strict: true, target: 'esnext', module: 'esnext', moduleResolution: 'bundler' };
const CONSUMER = { strict: true, target: 'esnext', module: 'esnext', moduleResolution: 'bundler', noEmit: true };

function config(compilerOptions: object, references: string[] = [], include = ['src']): string {
	return JSON.stringify({ compilerOptions, include, references: references.map((path) => ({ path })) });
}

const SHARED_SOURCE = 'export const value: number = 1;\nexport function greet(name: string): string { return name; }\n';
const SHARED_OUTPUT = 'export declare const value: string;\nexport declare function greet(name: number): number;\n';
const CONSUMER_SOURCE = 'import { value, greet } from "../../shared/src/index";\nconst read: string = value;\ngreet(1);\ngreet("one");\n';

test('an import of a referenced project whose output is not built is TS6305', async () => {
	const root = tempProject({
		'shared/tsconfig.json': config({ ...COMPOSITE, rootDir: 'src', outDir: 'out' }),
		'shared/src/index.ts': SHARED_SOURCE,
		'app/tsconfig.json': config(CONSUMER, ['../shared']),
		'app/src/main.ts': CONSUMER_SOURCE,
	});
	await expectSameAsTsc(root, ['-p', 'app/tsconfig.json'], { messages: true });
});

test('an import of a referenced project reads its output declaration under outDir', async () => {
	const root = tempProject({
		'shared/tsconfig.json': config({ ...COMPOSITE, rootDir: 'src', outDir: 'out' }),
		'shared/src/index.ts': SHARED_SOURCE,
		'shared/out/index.d.ts': SHARED_OUTPUT,
		'app/tsconfig.json': config(CONSUMER, ['../shared']),
		'app/src/main.ts': CONSUMER_SOURCE,
	});
	await expectSameAsTsc(root, ['-p', 'app/tsconfig.json']);
});

test('declarationDir wins over outDir, and the common source directory is the config directory without rootDir', async () => {
	const root = tempProject({
		'shared/tsconfig.json': config({ ...COMPOSITE, outDir: 'out', declarationDir: 'types' }),
		'shared/src/index.ts': SHARED_SOURCE,
		'shared/types/src/index.d.ts': SHARED_OUTPUT,
		'shared/out/src/index.d.ts': 'export declare const value: boolean;\n',
		'app/tsconfig.json': config(CONSUMER, ['../shared/tsconfig.json']),
		'app/src/main.ts': CONSUMER_SOURCE,
	});
	await expectSameAsTsc(root, ['-p', 'app/tsconfig.json']);
});

test('a referenced project without an output directory emits its declarations beside its sources', async () => {
	const root = tempProject({
		'shared/tsconfig.json': config(COMPOSITE),
		'shared/src/index.ts': SHARED_SOURCE,
		'shared/src/index.d.ts': SHARED_OUTPUT,
		'app/tsconfig.json': config(CONSUMER, ['../shared']),
		'app/src/main.ts': CONSUMER_SOURCE,
	});
	await expectSameAsTsc(root, ['-p', 'app/tsconfig.json']);
});

test('a project a reference references maps its sources too', async () => {
	const root = tempProject({
		'core/tsconfig.json': config({ ...COMPOSITE, rootDir: 'src', outDir: 'out' }),
		'core/src/index.ts': 'export const core: number = 1;\n',
		'core/out/index.d.ts': 'export declare const core: string;\n',
		'shared/tsconfig.json': config({ ...COMPOSITE, rootDir: 'src', outDir: 'out' }, ['../core']),
		'shared/src/index.ts': SHARED_SOURCE,
		'shared/out/index.d.ts': SHARED_OUTPUT,
		'app/tsconfig.json': config(CONSUMER, ['../shared']),
		'app/src/main.ts': `${CONSUMER_SOURCE}import { core } from "../../core/src/index";\nconst fromCore: number = core;\n`,
	});
	await expectSameAsTsc(root, ['-p', 'app/tsconfig.json']);
});

test("an output declaration's imports resolve from its source's directory", async () => {
	// tsc writes `../../core/src` as the source wrote it, which names nothing
	// from `shared/out/types/`, one directory deeper.
	const root = tempProject({
		'core/tsconfig.json': config({ ...COMPOSITE, rootDir: 'src', outDir: 'out' }),
		'core/src/index.ts': 'export class Counter { increment(): number { return 1; } }\n',
		'core/out/index.d.ts': 'export declare class Counter { increment(): string; }\n',
		'shared/tsconfig.json': config({ ...COMPOSITE, rootDir: 'src', outDir: 'out', declarationDir: 'out/types' }, ['../core']),
		'shared/src/index.ts': 'import { Counter } from "../../core/src";\nexport class NamedCounter extends Counter {}\n',
		'shared/out/types/index.d.ts': 'import { Counter } from "../../core/src";\nexport declare class NamedCounter extends Counter {}\n',
		'app/tsconfig.json': config(CONSUMER, ['../shared']),
		'app/src/main.ts': 'import { NamedCounter } from "../../shared/src";\nconst count: number = new NamedCounter().increment();\n',
	});
	await expectSameAsTsc(root, ['-p', 'app/tsconfig.json'], { messages: true });
});

test('a declaration file a referenced project includes is read as itself', async () => {
	const root = tempProject({
		'shared/tsconfig.json': config({ ...COMPOSITE, rootDir: 'src', outDir: 'out' }),
		'shared/src/index.ts': SHARED_SOURCE,
		'shared/src/ambient.d.ts': 'export declare const ambient: number;\n',
		'shared/out/index.d.ts': SHARED_OUTPUT,
		'app/tsconfig.json': config(CONSUMER, ['../shared']),
		'app/src/main.ts': 'import { ambient } from "../../shared/src/ambient";\nconst read: string = ambient;\n',
	});
	await expectSameAsTsc(root, ['-p', 'app/tsconfig.json']);
});
