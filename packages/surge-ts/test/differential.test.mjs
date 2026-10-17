// Differential tests: the same queries through typescript@6.0.3 (the last
// JavaScript compiler API) and surge-ts, on small projects written for each
// area. Every observation must agree: node kinds and positions from
// `forEachChild`, and for each identifier its symbol's name, flags and
// declarations and `getTypeOfSymbolAtLocation`, and `getTypeAtLocation` of
// its expressions.
// `scripts/api/compare-api.ts` measures the same over the repository's
// fixture projects.
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import typescript from "typescript-6";
import { describe, expect, test } from "vitest";
import surge from "../index.mjs";

const options = {
	strict: true,
	noEmit: true,
	target: 99,
	module: 99,
	moduleResolution: 100,
};

const FLAG_MASK =
	typescript.SymbolFlags.Variable |
	typescript.SymbolFlags.Property |
	typescript.SymbolFlags.Function |
	typescript.SymbolFlags.Class |
	typescript.SymbolFlags.Interface |
	typescript.SymbolFlags.Enum |
	typescript.SymbolFlags.EnumMember |
	typescript.SymbolFlags.Method |
	typescript.SymbolFlags.TypeParameter |
	typescript.SymbolFlags.TypeAlias |
	typescript.SymbolFlags.Alias;

function write(files) {
	const dir = realpathSync(mkdtempSync(path.join(tmpdir(), "surge-ts-diff-")));
	for (const [name, text] of Object.entries(files)) {
		const file = path.join(dir, name);
		mkdirSync(path.dirname(file), { recursive: true });
		writeFileSync(file, text);
	}
	return dir;
}

/** What a tool observes of one file through one implementation. */
function observe(ts, program, fileName) {
	const checker = program.getTypeChecker();
	const sourceFile = program.getSourceFile(fileName);
	const kindName = (kind) => ts.SyntaxKind[kind];
	// A default library's text is each implementation's own, so only the
	// kind of a declaration there is compared.
	const declarationKey = (declaration) => {
		const file = declaration.getSourceFile();
		return program.isSourceFileDefaultLibrary(file)
			? `lib ${kindName(declaration.kind)}`
			: `${path.basename(file.fileName)} ${kindName(declaration.kind)}@${declaration.getStart()}`;
	};
	const lines = [];
	ts.forEachChild(sourceFile, function visit(node) {
		const where = `${kindName(node.kind)}@${node.getStart()}-${node.end}`;
		lines.push(where);
		if (ts.isIdentifier(node)) {
			const symbol = checker.getSymbolAtLocation(node);
			if (symbol) {
				const type = checker.typeToString(checker.getTypeOfSymbolAtLocation(symbol, node));
				lines.push(`  ${node.text} -> ${symbol.getName()} [${symbol.flags & FLAG_MASK}]: ${type}`);
				lines.push(`    declarations ${(symbol.declarations ?? []).map(declarationKey).join(", ")}`);
			}
		}
		if (ts.isCallExpression(node) || ts.isPropertyAccessExpression(node) || ts.isBinaryExpression(node) || ts.isObjectLiteralExpression(node)) {
			lines.push(`  type ${checker.typeToString(checker.getTypeAtLocation(node))}`);
		}
		ts.forEachChild(node, visit);
	});
	return lines;
}

function compare(files, entry = "index.ts") {
	const dir = write(files);
	const fileName = path.join(dir, entry);
	const expected = observe(typescript, typescript.createProgram([fileName], options), fileName);
	const actual = observe(surge, surge.createProgram([fileName], options), fileName);
	expect(actual.join("\n")).toBe(expected.join("\n"));
}

describe("surge-ts agrees with typescript@6", () => {
	test("variables, literals and widening", () => {
		compare({
			"index.ts": "const a = 1;\nlet b = 'text';\nconst c = [1, 2, 3];\nlet d = a > 0 ? b : undefined;\nexport {};\n",
		});
	});

	test("functions, calls and return types", () => {
		compare({
			"index.ts":
				"function add(x: number, y: number): number { return x + y; }\nconst sum = add(1, 2);\nconst double = (value: number) => value * 2;\nconst doubled = double(sum);\nexport {};\n",
		});
	});

	test("interfaces, members and property access", () => {
		compare({
			"index.ts":
				"interface User { id: number; name: string; email?: string }\nconst user: User = { id: 1, name: 'Ada' };\nconst id = user.id;\nconst name = user.name.toUpperCase();\nexport {};\n",
		});
	});

	test("classes, members and instances", () => {
		compare({
			"index.ts":
				"class Counter {\n  count = 0;\n  increment(by: number): number { this.count += by; return this.count; }\n}\nconst counter = new Counter();\nconst next = counter.increment(1);\nexport {};\n",
		});
	});

	test("narrowing", () => {
		compare({
			"index.ts":
				"function measure(value: string | number): number {\n  if (typeof value === 'string') { return value.length; }\n  return value.valueOf();\n}\nexport {};\n",
		});
	});

	test("imports, exports and aliases", () => {
		compare(
			{
				"index.ts": "import { greet, version } from './lib';\nimport * as lib from './lib';\nconst message = greet('Ada');\nconst same = lib.version === version;\n",
				"lib.ts": "export function greet(name: string): string { return name; }\nexport const version = 1;\n",
			},
			"index.ts",
		);
	});

	test("enums and type aliases", () => {
		compare({
			"index.ts":
				"enum Color { Red, Green }\ntype Status = 'on' | 'off';\nconst color = Color.Green;\ndeclare let status: Status;\nconst on = status === 'on';\nexport {};\n",
		});
	});

	test("object literals and destructuring", () => {
		compare({
			"index.ts":
				"const point = { x: 1, y: 2 };\nconst { x, y } = point;\nconst total = x + y;\nconst moved = { ...point, x: 3 };\nexport {};\n",
		});
	});

	test("namespace, class and enum values", () => {
		compare({
			"index.ts":
				"namespace Settings { export const debug = true; }\nconst settings = Settings;\nclass Store { size = 0; }\nconst StoreClass = Store;\nenum Level { Low, High }\nconst levels = Level;\nexport {};\n",
		});
	});

	test("an optional parameter's type prints with `| undefined`", () => {
		compare({ "index.ts": "declare function pad(width?: number): string;\nconst padding = pad;\nexport {};\n" });
	});

	test("a declaration's initializer narrows its declared union type", () => {
		compare({ "index.ts": "type Status = 'on' | 'off';\nlet status: Status = 'on';\nconst copy = status;\nexport {};\n" });
	});

	test("members read through instantiations, primitives, arrays, unions and intersections keep their declarations", () => {
		compare({
			"index.ts":
				"interface Matchers { ok(): void }\ninterface Expectation<T> { readonly to: Expectation<T> & Matchers; equal(other: unknown): Expectation<T> }\ndeclare function expect<T>(value: T): Expectation<T> & Matchers;\nconst result = expect(1).to.equal(1);\nexpect('a').to.ok();\ninterface Box<T> { value: T }\ndeclare const either: Box<string> | Box<number>;\nconst value = either.value;\nconst list: number[] = [];\nlist.push(1);\nconst length = 'text'.length;\nexport {};\n",
		});
	});

	test("const contexts", () => {
		compare({
			"index.ts":
				"const pair = [1, 'a'] as const;\nconst config = { mode: 'dark', size: 2 } as const;\nconst mode = config.mode;\nexport {};\n",
		});
	});
});

// Each of these is a difference surge-ts has today, reduced to one construct
// and listed in docs/COMPILER_API.md. `test.fails` passes while the outputs
// differ, so fixing one fails its test here until it moves into the suite above.
describe("known differences from typescript@6", () => {
	test.fails("a tuple type alias keeps its name", () => {
		compare({ "index.ts": "type Pair = [string, number];\ndeclare const pair: Pair;\nexport {};\n" });
	});

	test.fails("an intersection operand keeps the alias it was written through", () => {
		compare({
			"index.ts":
				"type Matchers = Record<never, () => boolean>;\ninterface Expectation<T> { equal(other: unknown): Expectation<T> }\ndeclare function expect<T>(value: T): Expectation<T> & Matchers;\nconst expectation = expect(1);\nexport {};\n",
		});
	});

	test.fails("a union lists its members in the order their types were created", () => {
		compare({
			"index.ts": "type Status = 'on' | 'off';\ndeclare const status: Status;\nconst next = status === 'on' ? 'off' : 'on';\nexport {};\n",
		});
	});
});
