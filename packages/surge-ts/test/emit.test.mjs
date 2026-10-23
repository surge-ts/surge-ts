// Declaration emit: `Program.emit(target, writeFile, undefined, true)` writes
// the same `.d.ts` and `.d.ts.map` bytes as typescript@6.0.3, on projects
// shaped like the ones roblox-ts builds (CommonJS, node10 resolution, every
// file a module, `outDir` and `rootDir`).
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import typescript from "typescript-6";
import { describe, expect, test } from "vitest";
import surge from "../index.mjs";

function write(files) {
	const dir = realpathSync(mkdtempSync(path.join(tmpdir(), "surge-ts-emit-")));
	for (const [name, text] of Object.entries(files)) {
		const file = path.join(dir, name);
		mkdirSync(path.dirname(file), { recursive: true });
		writeFileSync(file, text);
	}
	return dir;
}

function optionsFor(dir, extra) {
	return {
		strict: true,
		target: 99,
		module: 1,
		moduleResolution: 2,
		moduleDetection: 3,
		declaration: true,
		declarationMap: true,
		rootDir: path.join(dir, "src"),
		outDir: path.join(dir, "out"),
		types: [],
		ignoreDeprecations: "6.0",
		...extra,
	};
}

function emitWith(ts, rootNames, options, customTransformers) {
	const program = ts.createProgram(rootNames, options);
	const outputs = [];
	for (const fileName of rootNames) {
		const result = program.emit(program.getSourceFile(fileName), (name, text) => outputs.push([name, text]), undefined, true, customTransformers);
		expect(result.emitSkipped).toBe(false);
	}
	return outputs;
}

function expectSameEmit(files, extra = {}, customTransformers = undefined) {
	const dir = write(files);
	const rootNames = Object.keys(files)
		.filter((name) => name.startsWith("src/"))
		.map((name) => path.join(dir, name));
	const options = optionsFor(dir, extra);
	const expected = emitWith(typescript, rootNames, options, customTransformers);
	expect(emitWith(surge, rootNames, options, customTransformers)).toEqual(expected);
	return expected;
}

describe("declaration emit", () => {
	test("literal constants, widened variables and inferred returns", () => {
		expectSameEmit({
			"src/index.ts": [
				"export const answer = 42;",
				'export const label = "new type";',
				"export const negative = -1;",
				"export const flag = true;",
				"export let total = 0;",
				"export const either = Math.random() > 0.5 ? 1 : 2;",
				"export function add(value: number) {",
				"\ttotal += value;",
				"\treturn total;",
				"}",
				"export function scale(value: number, multiplier: number) { return value * multiplier; }",
				"export function nothing() {}",
				"",
			].join("\n"),
		});
	});

	test("a constant re-exported through an import keeps its literal", () => {
		expectSameEmit({
			"src/value.ts": "export const value = 42;\n",
			"src/index.ts": 'import { value } from "./value"; export const answer = value;\n',
		});
	});

	test("classes, interfaces and type literals", () => {
		expectSameEmit({
			"src/index.ts": [
				"export interface Named {",
				"\tname: string;",
				"}",
				"",
				"export class Signal<T> {",
				"\tprivate listeners = new Set<(value: T) => void>();",
				"\tpublic value = 0;",
				"\tpublic connect(listener: (value: T) => void) {",
				"\t\tthis.listeners.add(listener);",
				"\t\treturn () => this.listeners.delete(listener);",
				"\t}",
				"\tpublic fire(value: T) {",
				"\t\tfor (const listener of this.listeners) { listener(value); }",
				"\t}",
				"}",
				'export type Outcome = { kind: "won"; reward: number } | { kind: "lost"; remaining: number };',
				"",
			].join("\n"),
		});
	});

	test("imports a declaration needs stay, the others go", () => {
		expectSameEmit({
			"src/types.ts": "export interface Item { id: number; label: string; }\nexport interface Enemy { id: number; }\n",
			"src/math.ts": "export function clamp(value: number, low: number, high: number) { return Math.max(low, Math.min(high, value)); }\n",
			"src/index.ts": [
				'import { clamp } from "./math";',
				'import type { Item, Enemy } from "./types";',
				"export function first(items: Array<Item>): Item { return items[clamp(0, 0, items.length)]; }",
				"",
			].join("\n"),
		});
	});

	test("an inferred type from a module the file does not import is an import type", () => {
		expectSameEmit({
			"src/core/types.ts": "export interface Biome { id: number; multiplier: number; }\n",
			"src/core/index.ts": 'export type { Biome } from "./types";\n',
			"src/catalog/entry.ts": 'import type { Biome } from "../core";\nexport const entry: Biome = { id: 0, multiplier: 1 };\n',
			"src/catalog/index.ts": 'import { entry } from "./entry";\nexport const biomes = [entry];\n',
		});
	});

	test("a module with no exports gets `export {}`", () => {
		expectSameEmit({
			"src/lib.ts": "export const answer = 42;\n",
			"src/main.ts": 'import { answer } from "./lib"; if (answer !== 42) throw "unexpected";\n',
		});
	});

	test("re-exports and local export lists", () => {
		expectSameEmit({
			"src/helper.ts": "export const value = 42;\nexport class Counter { public count = 0; }\n",
			"src/index.ts": [
				'export { value } from "./helper";',
				'export * as helper from "./helper";',
				'import { Counter } from "./helper";',
				"const hidden = 1;",
				"export { hidden, Counter };",
				"",
			].join("\n"),
		});
	});

	test("an afterDeclarations transformer walks the declarations with typescript's visitEachChild", () => {
		// A roblox-ts plugin's shape: typescript's own visitor and factory over
		// the declaration file, here rewriting a module specifier.
		const rewrite = (context) => (source) => {
			const visit = (node) =>
				typescript.isStringLiteral(node) && node.text === "./target"
					? typescript.factory.createStringLiteral("./other")
					: typescript.visitEachChild(node, visit, context);
			return typescript.visitNode(source, visit);
		};
		const outputs = expectSameEmit(
			{
				"src/index.ts": 'export { value } from "./target";\nexport interface Shape { size: number; }\n',
				"src/target.ts": "export const value = 1;\n",
				"src/other.ts": "export const value = 2;\n",
			},
			{},
			{ afterDeclarations: [rewrite] },
		);
		expect(outputs.find(([name]) => name.endsWith("index.d.ts"))[1]).toContain('"./other"');
	});

	test("a class expression is a constructor type literal", () => {
		expectSameEmit({ "src/other.ts": "export const Other = class { value = 1; readonly label = 'x'; };\n" });
	});

	test("a private member of an exported class expression is an error that blocks the emit", () => {
		const dir = write({
			"src/hidden.ts": "export const Hidden = class { private value = 1; };\n",
			"src/index.ts": "export const value = 2;\n",
		});
		const rootNames = [path.join(dir, "src/hidden.ts"), path.join(dir, "src/index.ts")];
		const shape = (diagnostic) => ({
			file: diagnostic.file && path.basename(diagnostic.file.fileName),
			start: diagnostic.start,
			length: diagnostic.length,
			code: diagnostic.code,
			category: diagnostic.category,
			messageText: diagnostic.messageText,
			relatedInformation: diagnostic.relatedInformation?.map(shape),
		});
		const observe = (ts, extra) => {
			const program = ts.createProgram(rootNames, optionsFor(dir, extra));
			const outputs = [];
			const results = rootNames.map((fileName) => {
				const result = program.emit(program.getSourceFile(fileName), (name, text) => outputs.push([name, text]), undefined, true);
				return { emitSkipped: result.emitSkipped, diagnostics: result.diagnostics.map(shape) };
			});
			const preEmit = ts.getPreEmitDiagnostics(program, program.getSourceFile(rootNames[0])).map(shape);
			return { outputs, results, preEmit, declaration: program.getDeclarationDiagnostics().map(shape) };
		};
		// options both accept without a word, so that noEmitOnError weighs only the declaration error
		for (const extra of [{ module: 99, moduleResolution: 100, ignoreDeprecations: undefined }, { module: 99, moduleResolution: 100, ignoreDeprecations: undefined, noEmitOnError: true }]) {
			const expected = observe(typescript, extra);
			expect(expected.declaration.map((diagnostic) => diagnostic.code)).toEqual([4094]);
			expect(observe(surge, extra)).toEqual(expected);
		}
	});

	test("without declarationMap there is no map and no sourceMappingURL", () => {
		const outputs = expectSameEmit({ "src/index.ts": "export const value = 1;\n" }, { declarationMap: false });
		expect(outputs.map(([name]) => path.basename(name))).toEqual(["index.d.ts"]);
	});
});
