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

	test("an alias of an instantiation keeps its name, an intersection operand included", () => {
		compare({
			"index.ts":
				"type Matchers = Record<never, () => boolean>;\ninterface Expectation<T> { equal(other: unknown): Expectation<T> }\ndeclare function expect<T>(value: T): Expectation<T> & Matchers;\nconst expectation = expect(1);\ntype Flags = Record<'a', number>;\ndeclare const flags: Flags[];\ntype Loose = Partial<{ a: number }>;\ndeclare const loose: Loose[];\nexport {};\n",
		});
	});

	test("an intersection alias that reduces to one operand is that operand", () => {
		compare({
			"index.ts":
				"interface Options { lazy?: { enabled: boolean } }\ntype Lazy = Required<NonNullable<Options['lazy']>>;\ndeclare const lazy: Lazy;\ninterface User { name: string }\ndeclare const user: NonNullable<User | undefined>;\ndeclare const literal: NonNullable<{ a: 1 } | null>;\ntype WithId<T> = T & { id: string };\ndeclare const withId: WithId<User>;\ndeclare function keep<T>(value: NonNullable<T>): void;\nexport {};\n",
		});
	});

	test("a union written with a named union keeps its name", () => {
		compare({
			"index.ts":
				"type Filter = 'all' | 'active';\ndeclare const filter: Filter | 'none';\ndeclare const maybe: Filter | undefined;\nconst present = maybe!;\nconst fallback = maybe ?? 'none';\nfunction pick(value: Filter | null) { if (value !== null) { return value; } return undefined; }\nexport {};\n",
		});
	});

	test("a union lists its members in the order their types were created", () => {
		compare({
			"index.ts": "type Status = 'on' | 'off';\ndeclare const status: Status;\nconst next = status === 'on' ? 'off' : 'on';\nexport {};\n",
		});
	});
});

/** What a tool built on the type internals reads off one file. */
function observeInternals(ts, program, fileName) {
	const checker = program.getTypeChecker();
	const sourceFile = program.getSourceFile(fileName);
	const show = (type) => (type ? checker.typeToString(type) : "-");
	const lines = [];
	const visit = (node) => {
		const at = `${ts.SyntaxKind[node.kind]}@${node.getStart(sourceFile)}`;
		if (ts.isClassDeclaration(node) || ts.isInterfaceDeclaration(node)) {
			const declared = checker.getDeclaredTypeOfSymbol(checker.getSymbolAtLocation(node.name));
			lines.push(`${at} bases ${declared.getBaseTypes()?.map(show).join(", ")}`);
		}
		if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
			const type = checker.getTypeAtLocation(node.name);
			if (checker.isTupleType(type)) {
				const target = type.target;
				lines.push(
					`${at} tuple [${Array.from(target.elementFlags)}] min ${target.minLength} fixed ${target.fixedLength} rest ${target.hasRestElement} combined ${target.combinedFlags} readonly ${target.readonly} args ${checker.getTypeArguments(type).map(show)}`,
				);
			}
		}
		if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
			const signature = checker.getResolvedSignature(node);
			lines.push(`${at} resolved ${signature && checker.signatureToString(signature)} returns ${signature && show(signature.getReturnType())}`);
			for (const argument of node.arguments ?? []) lines.push(`${at} argument context ${show(checker.getContextualType(argument))}`);
		}
		if (ts.isFunctionDeclaration(node) && node.name) {
			const signature = checker.getTypeAtLocation(node.name).getCallSignatures()[0];
			lines.push(`${at} this ${signature.thisParameter ? show(checker.getTypeOfSymbol(signature.thisParameter)) : "-"} parameters ${signature.parameters.map((parameter) => parameter.name)}`);
		}
		if ((ts.isArrayLiteralExpression(node) || ts.isObjectLiteralExpression(node)) && isDestructuringTarget(ts, node)) {
			lines.push(`${at} pattern ${show(checker.getTypeOfAssignmentPattern(node))}`);
		}
		for (const predicate of ["isExpression", "isLeftHandSideExpression", "isCallLikeExpression", "isCallChain", "isPropertyAccessChain", "isNullishCoalesce", "isToken"]) {
			if (ts[predicate](node)) lines.push(`${at} ${predicate}`);
		}
		const name = ts.getNameOfDeclaration(node);
		if (name) lines.push(`${at} named ${name.getText(sourceFile)}`);
		ts.forEachChild(node, visit);
	};
	// Checking binds the file: TypeScript sets parents then.
	program.getSemanticDiagnostics(sourceFile);
	visit(sourceFile);
	return lines;
}

/** An object or array literal written where it is assigned to: a destructuring pattern. */
function isDestructuringTarget(ts, node) {
	const parent = node.parent;
	if (ts.isBinaryExpression(parent)) return parent.left === node && parent.operatorToken.kind === ts.SyntaxKind.EqualsToken;
	if (ts.isForOfStatement(parent)) return parent.initializer === node;
	if (ts.isPropertyAssignment(parent)) return parent.initializer === node && isDestructuringTarget(ts, parent.parent);
	if (ts.isArrayLiteralExpression(parent)) return isDestructuringTarget(ts, parent);
	return false;
}

function compareInternals(files, entry = "index.ts") {
	const dir = write(files);
	const fileName = path.join(dir, entry);
	const expected = observeInternals(typescript, typescript.createProgram([fileName], options), fileName);
	const actual = observeInternals(surge, surge.createProgram([fileName], options), fileName);
	expect(actual.join("\n")).toBe(expected.join("\n"));
}

describe("type internals agree with typescript@6", () => {
	test("base types of classes and interfaces", () => {
		compareInternals({
			"index.ts":
				"interface Base { a: number }\ninterface Other<T> { b: T }\ninterface Derived extends Base, Other<string> { c: boolean }\nclass Animal { name = '' }\nclass Dog extends Animal { bark() {} }\nclass Box<T> { value!: T }\nclass NumberBox extends Box<number> {}\nexport {};\n",
		});
	});

	test("tuple targets", () => {
		compareInternals({
			"index.ts":
				"declare const open: [number, string?, ...boolean[]];\ndeclare const fixed: [number, string];\ndeclare const frozen: readonly [number, number];\nexport {};\n",
		});
	});

	test("resolved signatures and argument contexts", () => {
		compareInternals({
			"index.ts":
				"declare function identity<T>(value: T): T;\ndeclare function pick(value: number): number;\ndeclare function pick(value: string): string;\ndeclare function rest(first: number, ...others: string[]): void;\nconst one = identity(1);\nconst text = pick('a');\nrest(1, 'a', 'b');\nexport {};\n",
		});
	});

	test("a generic global's call is instantiated", () => {
		compareInternals(
			{
				"globals.d.ts": "declare const expectValue: { <T>(value: T): { got: T } };\ndeclare function wrap<T>(value: T): { got: T };\n",
				"augment.d.ts": "export {};\ndeclare global { const expectGlobal: { <T>(value: T): { got: T }; extend(matchers: object): void } }\n",
				"index.ts": "/// <reference path='globals.d.ts' />\n/// <reference path='augment.d.ts' />\nconst a = expectValue(1);\nconst b = wrap('x');\nconst c = expectGlobal(true);\nexport {};\n",
			},
		);
	});

	test("this parameters", () => {
		compareInternals({
			"index.ts": "class Animal { name = '' }\ndeclare function speak(this: Animal, loud: boolean): void;\ndeclare function plain(loud: boolean): void;\nexport {};\n",
		});
	});

	test("destructuring assignment patterns", () => {
		compareInternals({
			"index.ts":
				"let a = 0;\nlet b = '';\nlet c = false;\ndeclare const pair: [number, string];\n[a, b] = pair;\ndeclare const flags: { c: boolean };\n({ c } = flags);\ndeclare const nested: { inner: [number, string] };\n({ inner: [a, b] } = nested);\nfor ([a, b] of [pair]) {}\nexport {};\n",
		});
	});

	test("syntax predicates and declaration names", () => {
		compareInternals({
			"index.ts":
				"declare const o: { f?(): number; p?: { q: number } } | undefined;\nconst x = o?.f?.() ?? o?.p?.q;\nconst named = () => 1;\nconst object = { method: function () {}, arrow: () => 2 };\nclass C { static s = 1; m() { return new C(); } }\nexport default C;\n",
		});
	});
});

// What roblox-ts's transformer reads to choose its Luau: an answer that
// differs from typescript's changes the code it writes without an error.
describe("transformer queries agree with typescript@6", () => {
	function bothPrograms(files, roots, extraOptions = {}) {
		const dir = write(files);
		const rootNames = roots.map((root) => path.join(dir, root));
		return [typescript, surge].map((ts) => {
			const program = ts.createProgram(rootNames, { ...options, ...extraOptions });
			return { ts, program, checker: program.getTypeChecker(), file: (name) => program.getSourceFile(path.join(dir, name)) };
		});
	}

	test("postfix and prefix update expressions are numbers", () => {
		const results = bothPrograms({ "index.ts": "let index = 0;\nconst post = index++;\nconst pre = --index;\nlet big = 1n;\nconst bigPost = big--;\nexport {};\n" }, ["index.ts"]).map(
			({ ts, checker, file }) =>
				file("index.ts")
					.statements.filter(ts.isVariableStatement)
					.map((statement) => statement.declarationList.declarations[0])
					.filter((declaration) => declaration.initializer && !ts.isLiteralExpression(declaration.initializer))
					.map((declaration) => `${declaration.name.text}: ${checker.typeToString(checker.getTypeAtLocation(declaration.initializer))}`),
		);
		expect(results[1]).toEqual(results[0]);
	});

	test("a nested array binding pattern over an iterable is the iterated element", () => {
		const source =
			"declare const pairs: Set<[number, [string, boolean]]>;\nconst [[n, [s, b]], ...others] = pairs;\ndeclare const tuples: [number, string][];\nconst [[x, y], ...more] = tuples;\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isArrayBindingPattern(node)) lines.push(`${node.getText()}: ${checker.typeToString(checker.getTypeAtLocation(node))}`);
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a method argument's contextual type is its parameter in the receiver's instantiation", () => {
		const source =
			"const list: string[] = [];\nlist.push('x');\ninterface Box<T> { put(value: T): void; many(...values: T[]): void }\ndeclare const box: Box<number>;\nbox.put(1);\nbox.many(1, 2);\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isCallExpression(node)) {
					for (const argument of node.arguments) {
						const contextual = checker.getContextualType(argument);
						lines.push(`${argument.getText()}: ${contextual ? checker.typeToString(contextual) : "(none)"}`);
					}
					const signature = checker.getTypeAtLocation(node.expression).getCallSignatures()[0];
					lines.push(signature.getParameters().map((parameter) => `${parameter.name}: ${checker.typeToString(checker.getTypeOfSymbol(parameter))}`).join(", "));
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a generic call argument's contextual type is its inferred parameter, with that as its base constraint", () => {
		const globals =
			"export {};\ndeclare global {\n\tinterface Expectation<T> { readonly to: Expectation<T>; equal(other: unknown): void }\n\tconst expect: { <T>(value: T): Expectation<T>; extend(other: object): void };\n}\n";
		const source =
			"declare const local: { <T>(value: T): { equal(other: T): void } };\ndeclare function pair(): [number, string] | undefined;\nlocal(pair()?.[1]).equal('2');\nexpect(pair()?.[1]).to.equal('2');\nexpect(pair).to.equal(undefined);\nfunction id<T>(value: T): T { return value; }\nid(1);\nexport {};\n";
		const results = bothPrograms({ "globals.d.ts": globals, "index.ts": source }, ["globals.d.ts", "index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isCallExpression(node)) {
					for (const argument of node.arguments) {
						const contextual = checker.getContextualType(argument);
						let constraint;
						try {
							const base = contextual && checker.getBaseConstraintOfType(contextual);
							constraint = base ? checker.typeToString(base) : "(none)";
						} catch (error) {
							constraint = `throws ${error.message}`;
						}
						lines.push(`${argument.getText()}: ${contextual ? checker.typeToString(contextual) : "(none)"} constraint ${constraint}`);
					}
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an element access on a tuple intersection, optional or not, is that element", () => {
		const source =
			"type LuaTuple<T extends Array<any>> = T & { readonly _nominal_LuaTuple: unique symbol };\ndeclare function pair(): LuaTuple<[number, string]> | undefined;\ndeclare function nested(): LuaTuple<[number, () => LuaTuple<[string, () => number]>]> | undefined;\ndeclare const plain: LuaTuple<[number, string]>;\nconst a = pair()?.[1];\nconst b = nested()?.[1]?.()?.[1]?.();\nconst c = plain[0];\nconst d = nested?.()?.[1];\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isElementAccessExpression(node) || ts.isCallExpression(node)) lines.push(`${node.getText()}: ${checker.typeToString(checker.getTypeAtLocation(node))}`);
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	/**
	 * What roblox-ts's `isMethodFromType` reads off a callee to choose `a:f()`
	 * or `a.f()`: the type's symbol, and each call signature's declaration
	 * kind and `this` parameter.
	 */
	function observeCallees(ts, checker, sourceFile) {
		const lines = [];
		ts.forEachChild(sourceFile, function visit(node) {
			if (ts.isCallExpression(node)) {
				const callee = node.expression;
				const type = checker.getNonNullableType(checker.getTypeAtLocation(callee));
				const members = type.isUnion() ? type.types : [type];
				const described = members.map((member) => {
					const signatures = member.getCallSignatures().map((signature) => {
						const receiver = signature.thisParameter ? checker.typeToString(checker.getTypeOfSymbolAtLocation(signature.thisParameter, callee)) : "-";
						return `${signature.declaration ? ts.SyntaxKind[signature.declaration.kind] : "none"}/this=${receiver}`;
					});
					return `${member.symbol ? member.symbol.name : "(no symbol)"} [${signatures.join(", ")}]`;
				});
				lines.push(`${callee.getText()}: ${described.join(" | ")}`);
			}
			ts.forEachChild(node, visit);
		});
		return lines;
	}

	test("a callee's symbol, declaration and receiver, as roblox-ts reads a method off them", () => {
		const source = [
			"type Callable<T> = (this: T, value: number) => number;",
			"function make<T extends {}>() { const callback: Callable<T> = (value) => value + 1; return { callback }; }",
			"const made = make<{}>();",
			"made.callback(41);",
			"const list: { onEnd: Array<(this: {}, arg?: string) => void> } = { onEnd: [function () {}] };",
			"let n = 0;",
			"list.onEnd[n]('foo');",
			"list.onEnd[n]?.('foo');",
			"class Base { public $() { return 1; } }",
			"class Derived extends Base { public $() { return super.$(); } }",
			"const derived = new (class extends class { $() { return 1; } } { $() { return super.$(); } })();",
			"derived.$();",
			"const object = { method(value: number) { return value; }, [`${n}`](value: number) { return value; } };",
			"object.method(1);",
			"object[`${n}`](1);",
			"let i = 0;",
			"const numbered = { id: ++i, [++i](value: 5) { return value; }, f(value: 5) { return value; } };",
			"const holder = { k: numbered };",
			"numbered[i](5);",
			"holder.k[i](5);",
			"interface Numerable { n: number }",
			"function add(this: Numerable, value: number) { return (this.n += value); }",
			"const mul = function <T extends Numerable>(this: T, value: number) { this.n *= value; return this; };",
			"const fluent = { add, n: 10, mul };",
			"fluent.mul(3).add(6);",
			"function local() { let i = 0; class A { [++i]() { return 'first'; } } const a = new A(); (a[1 as never] as (this: A) => string)(); }",
			"export {};",
		].join("\n");
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => observeCallees(ts, checker, file("index.ts")));
		expect(results[1]).toEqual(results[0]);
	});

	test("an object literal's computed method is its own signature, a method", () => {
		const source = "let i = 0;\nconst key = () => 'k';\nconst o = { [++i](n: 5) { return 1; }, async [`${++i}`]() {}, [key()]() {}, f(n: 5) { return 2; } };\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isMethodDeclaration(node)) {
					const type = checker.getTypeAtLocation(node);
					const declarations = type.getCallSignatures().map((signature) => (signature.declaration ? ts.SyntaxKind[signature.declaration.kind] : "none"));
					lines.push(`${node.name.getText()}: ${checker.typeToString(type)} symbol=${type.symbol?.name} [${declarations.join(", ")}]`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an array method read off a union of tuples has the array method's symbol", () => {
		const source = "declare const tuples: [number, number, ...string[]] | [];\ntuples.pop();\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const names = new Set();
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isCallExpression(node)) {
					const type = checker.getTypeAtLocation(node.expression);
					for (const member of type.isUnion() ? type.types : [type]) names.add(member.symbol?.name ?? "(no symbol)");
				}
				ts.forEachChild(node, visit);
			});
			return [...names];
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a type node in a function body names the types that body declares", () => {
		const source =
			"function outer() {\n\tclass A { z = 1; h(this: A, n: number) {} }\n\tinterface I { x: number }\n\ttype T = { y: number };\n\tconst fa: (this: A) => void = () => {};\n\tconst fi: (this: I) => void = () => {};\n\tconst ft: (this: T) => void = () => {};\n\tconst va: A = new A();\n}\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isTypeReferenceNode(node) || (ts.isParameter(node) && node.name.getText() === "this")) {
					lines.push(`${ts.SyntaxKind[node.kind]} ${node.getText()}: ${checker.typeToString(checker.getTypeAtLocation(node))}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an intrinsic type is one object, getAnyType()'s; an auto-typed variable's is its own", () => {
		const source =
			"declare function print(value: unknown): void;\nfunction foo(x: any, y: { [k: string]: any }) { print(x.y); print(y.z()); }\nconst values: number[] = [];\nfor (let i; (i = values.length) < 2; i++) { values.push(i); }\nconst o = { 5: 1, no() { return (this[5] *= 7); } };\nlet k = 0;\nconst nested = { jk: { b: k++, g() { this.b++; }, no: function () { return (this[5] *= 7); }, 5: 3 } }.jk.no();\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const any = checker.getAnyType();
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isIdentifier(node) || ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node) || ts.isCallExpression(node)) {
					const type = checker.getTypeAtLocation(node);
					if (type.flags & ts.TypeFlags.Any || ts.isElementAccessExpression(node)) lines.push(`${node.getText()}: ${checker.typeToString(type)} anyType=${type === any}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a function read through a variable, a typeof alias, an object pattern or an invoked arrow has the function's symbol", () => {
		const source =
			"declare function pick(a: string): string;\ndeclare function pick(a: number): number;\nconst viaVariable = pick;\ntype Helper = typeof pick;\ndeclare const viaAlias: Helper;\nconst values = [1, 2, 3];\nconst { push } = values;\nconst invoked: typeof pick | undefined = (() => pick)();\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isCallExpression(node) && ts.isVariableDeclaration(node.parent)) {
					const type = checker.getTypeAtLocation(node);
					lines.push(`${node.getText()}: symbol=${type.symbol?.name}`);
				}
				if ((ts.isVariableDeclaration(node) || ts.isBindingElement(node)) && ts.isIdentifier(node.name) && !["values", "invoked"].includes(node.name.text)) {
					const type = checker.getTypeAtLocation(node.name);
					const declarations = type.symbol?.declarations?.map((declaration) => ts.SyntaxKind[declaration.kind]).join(",");
					const signatures = type.getCallSignatures().map((signature) => (signature.declaration ? ts.SyntaxKind[signature.declaration.kind] : "none"));
					lines.push(`${node.name.text}: symbol=${type.symbol?.name} [${declarations}] signatures=${signatures.join(",")}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an arrow takes its contextual `this` only when it is context-sensitive", () => {
		const source =
			"declare function accept(callback: (this: { n: number }, value: number) => number): void;\naccept((value: number) => value);\naccept((value) => value);\naccept(() => 1);\naccept(function (value: number) { return this.n + value; });\naccept(function (this: { n: number }, value: number) { return value; });\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) {
					const signature = checker.getTypeAtLocation(node).getCallSignatures()[0];
					const thisType = signature.thisParameter && checker.getTypeOfSymbolAtLocation(signature.thisParameter, node);
					lines.push(`${node.getText()}: this ${thisType ? checker.typeToString(thisType) : "(none)"}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a module's exports under `export =` are the assigned symbol's, joined by its other exports", () => {
		const files = {
			"klass.ts": "class Box { static make() { return new Box(); } }\nexport = Box;\n",
			"merged.ts": "function f() {}\nnamespace f { export const x = 1; }\nexport = f;\n",
			"value.ts": "const value = 42;\nexport = value;\n",
			"index.ts":
				"// @ts-nocheck\nnamespace N { export class Value { static s = 1; } export = Value; }\nnamespace M { export function Value() {} Value.property = 42; export = Value; }\nnamespace O { export const other = 1; export namespace Inner { export const y = 2; } export = Inner; }\nexport {};\n",
		};
		const results = bothPrograms(files, Object.keys(files)).map(({ ts, checker, file }) => {
			const lines = [];
			for (const name of ["klass.ts", "merged.ts", "value.ts"]) {
				const symbol = checker.getSymbolAtLocation(file(name));
				lines.push(`${name}: ${checker.getExportsOfModule(symbol).map((s) => s.name).sort().join(",")}`);
			}
			for (const statement of file("index.ts").statements.filter(ts.isModuleDeclaration)) {
				const symbol = checker.getSymbolAtLocation(statement.name);
				lines.push(`${statement.name.text}: ${checker.getExportsOfModule(symbol).map((s) => s.name).sort().join(",")}`);
			}
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a member read by a literal-typed key has the member's type, its symbol and declaration", () => {
		const source =
			"interface API {\n  callback(this: void, value: number): void;\n  method: (this: API, value: number) => void;\n}\ndeclare const object: API;\ndeclare const callbackKey: 'callback';\ndeclare const methodKey: 'method';\nobject[callbackKey](123);\nobject[methodKey](123);\nobject['method'](123);\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isElementAccessExpression(node)) {
					const type = checker.getTypeAtLocation(node);
					const signature = type.getCallSignatures()[0];
					const thisType = signature?.thisParameter && checker.getTypeOfSymbolAtLocation(signature.thisParameter, node);
					lines.push(`${node.getText()}: symbol=${type.symbol?.name} this=${thisType ? checker.typeToString(thisType) : "(none)"} declaration=${signature?.declaration ? ts.SyntaxKind[signature.declaration.kind] : "(none)"}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a file's triple-slash references are its referencedFiles, typeReferenceDirectives and libReferenceDirectives", () => {
		const text = '/// <reference path="./other.ts" />\n/// <reference types="node" resolution-mode="import" />\n/// <reference lib="es2020" />\n/// <reference types="types" preserve="true" />\nexport {};\n';
		const fields = [typescript, surge].map((ts) => {
			const file = ts.createSourceFile("index.ts", text, 99, true);
			return {
				referencedFiles: file.referencedFiles,
				typeReferenceDirectives: file.typeReferenceDirectives,
				libReferenceDirectives: file.libReferenceDirectives,
				hasNoDefaultLib: file.hasNoDefaultLib,
			};
		});
		expect(fields[1]).toEqual(fields[0]);
	});

	test("an array type's symbol is the global Array or ReadonlyArray; a tuple has none", () => {
		const source =
			"interface Numbers extends Array<number> {}\ndeclare const ro: readonly number[];\ndeclare const tup: [number, string];\ndeclare const roTup: readonly [number];\ndeclare const written: Array<string>;\ndeclare const numbers: Numbers;\nconst literal = [1, 'a'];\nconst nested = [[1]];\nconst mapped = [1, 2].map((n) => String(n));\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isVariableDeclaration(node)) lines.push(`${node.name.text}: ${checker.getTypeAtLocation(node.name).symbol?.name}`);
				if (ts.isArrayLiteralExpression(node)) lines.push(`${node.getText()}: ${checker.getTypeAtLocation(node).symbol?.name}`);
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a well-known symbol member has one name on the types that declare or instantiate it", () => {
		const source = "declare const set: Set<number>;\ndeclare const iterator: IterableIterator<number>;\ndeclare function generate(): Generator<number>;\nconst generated = generate();\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const sourceFile = file("index.ts");
			const iterable = checker.getDeclaredTypeOfSymbol(checker.resolveName("Iterable", sourceFile, ts.SymbolFlags.Type, false));
			const name = String(iterable.getProperties()[0].escapedName);
			const lines = [`Iterable: ${name.replace(/@\d+$/, "@N")}`];
			ts.forEachChild(sourceFile, function visit(node) {
				if (ts.isIdentifier(node) && ["set", "iterator", "generated"].includes(node.text) && ts.isVariableDeclaration(node.parent)) {
					const properties = checker.getTypeAtLocation(node).getProperties();
					lines.push(`${node.text}: ${properties.some((property) => property.escapedName === name)}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a namespace's export specifier puts its name in scope; an ambient module's does not", () => {
		const source =
			"declare function print(value: unknown): void;\nnamespace N { const value = 42; export { value as alias }; print(alias); }\ndeclare module 'm' { const inner: number; export { inner as outer }; const read: typeof outer; }\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isIdentifier(node) && (node.text === "alias" || node.text === "outer") && !ts.isExportSpecifier(node.parent)) {
					const symbol = checker.getSymbolAtLocation(node);
					const target = symbol && symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
					lines.push(`${node.text}: ${symbol ? `${symbol.name} alias=${!!(symbol.flags & ts.SymbolFlags.Alias)} -> ${target?.name}` : "(none)"}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("the base constraint of a union or intersection with type parameters", () => {
		const source =
			"interface Tagged { tag: string }\nfunction h<T, U extends string, V extends Tagged>(a: T | number, b: U | number, c: T & { n: 1 }, d: T & U, e: V & { length: number }) {}\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) =>
			file("index.ts")
				.statements.filter(ts.isFunctionDeclaration)
				.flatMap((declaration) => declaration.parameters)
				.map((parameter) => {
					const type = checker.getTypeAtLocation(parameter.name);
					const constraint = checker.getBaseConstraintOfType(type);
					const members = type.isUnionOrIntersection() ? type.types.map((member) => checker.typeToString(member)).sort().join(", ") : "";
					return `${parameter.name.text}: ${constraint ? checker.typeToString(constraint) : "(none)"} [${members}]`;
				}),
		);
		expect(results[1]).toEqual(results[0]);
	});

	test("a binding element named by a pattern, over a tuple intersection and in a for-of head", () => {
		const source =
			"type Tagged<T> = T & { tag: 1 };\ndeclare function wrapped(): Tagged<[[number, string], boolean]>;\nconst [[a, b], c] = wrapped();\ndeclare const pairs: Array<Tagged<[string, number]>>;\nfor (const [k, v] of pairs) {}\ndeclare const map: Map<string, number>;\nfor (const [key, value] of map) {}\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isArrayBindingPattern(node) || ts.isBindingElement(node)) lines.push(`${node.getText()}: ${checker.typeToString(checker.getTypeAtLocation(node))}`);
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an assignment pattern over an iterable reads the iterated element", () => {
		const source =
			"let a: string;\nlet b: number;\nconst obj = { x: new Map([['k', 1]]) };\n({ x: [[a, b]] } = obj);\ndeclare const it: Iterable<[number, [number]]>;\nlet c = 0;\nlet d = 0;\nfor ([c, [d]] of it) {}\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if ((ts.isArrayLiteralExpression(node) || ts.isObjectLiteralExpression(node)) && ts.isAssignmentTarget(node)) {
					lines.push(`${node.getText()}: ${checker.typeToString(checker.getTypeOfAssignmentPattern(node))}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an intersection alias read through a signature's return keeps its alias, arguments and unique member", () => {
		const source =
			"type Tagged<T> = T & { readonly tag: unique symbol };\ninterface Producer<T> { (): T }\ndeclare const produce: Producer<Tagged<[value: boolean]>>;\nfunction inferred() { return [false] as Tagged<[boolean]>; }\nconst value = inferred();\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const statements = file("index.ts").statements;
			const alias = statements.find(ts.isTypeAliasDeclaration);
			const nominal = checker.getTypeAtLocation(alias).getProperty("tag");
			const produce = statements.filter(ts.isVariableStatement)[0].declarationList.declarations[0];
			const returned = checker.getTypeAtLocation(produce.name).getCallSignatures()[0].getReturnType();
			const value = statements.filter(ts.isVariableStatement)[1].declarationList.declarations[0];
			const inferred = checker.getTypeAtLocation(value.name);
			return [
				`${returned.aliasSymbol?.name} ${(returned.aliasTypeArguments ?? []).map((argument) => checker.typeToString(argument)).join(", ")}`,
				`same tag through the return: ${returned.getProperty("tag") === nominal}`,
				`same tag through an inferred return: ${inferred.getProperty("tag") === nominal}`,
			];
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a class's base read off an extends clause naming a namespace's re-export", () => {
		const files = {
			"base.d.ts": "declare abstract class Base<P = {}> {\n  readonly props: P;\n  render(): string;\n}\nexport = Base;\n",
			"lib.d.ts": "import Base from './base';\ndeclare namespace Lib {\n  export { Base };\n}\nexport = Lib;\n",
			"index.ts": "import Lib from './lib';\nclass View extends Lib.Base {\n  render() { return 'x'; }\n}\nexport {};\n",
		};
		const results = bothPrograms(files, ["index.ts"], { allowSyntheticDefaultImports: true }).map(({ ts, checker, file }) => {
			const view = file("index.ts").statements.find(ts.isClassDeclaration);
			const heritage = view.heritageClauses[0].types[0];
			const aliased = checker.getAliasedSymbol(checker.getSymbolAtLocation(heritage.expression));
			const base = checker.getTypeAtLocation(heritage);
			const render = checker.getTypeOfPropertyOfType(base, "render");
			const declaration = render.getCallSignatures()[0].getDeclaration();
			return [
				`${aliased.name} ${ts.SyntaxKind[aliased.declarations[0].kind]}`,
				`base symbol ${base.symbol?.name}`,
				`render declared by ${declaration ? ts.SyntaxKind[declaration.kind] : "nothing"}`,
			];
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a callable generic interface's instantiation a declared method returns keeps its type arguments", () => {
		const files = {
			"steps.d.ts":
				"type Tagged<T> = T & { tag: 1 };\ninterface Step<T> extends Iterable<T> { (): T }\ninterface String {\n  steps(this: string): Step<Tagged<string[]>>;\n}\n",
			"index.ts": "const iterator = 'x'.steps();\nexport {};\n",
		};
		const results = bothPrograms(files, ["index.ts", "steps.d.ts"]).map(({ ts, checker, file }) => {
			const declaration = file("index.ts").statements.filter(ts.isVariableStatement)[0].declarationList.declarations[0];
			const type = checker.getTypeAtLocation(declaration.name);
			return [
				`reference ${(type.objectFlags & ts.ObjectFlags.Reference) !== 0}`,
				checker.getTypeArguments(type).map((argument) => checker.typeToString(argument)).join(", "),
			];
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a tuple's number index is its elements' union, an intersection's its tuple operand's", () => {
		const source =
			"declare const t: [number, string];\ndeclare const o: [number, ...string[]];\ntype Tagged<T> = T & { tag: 1 };\ndeclare const l: Tagged<[boolean, number]>;\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) =>
			file("index.ts")
				.statements.filter(ts.isVariableStatement)
				.map((statement) => statement.declarationList.declarations[0])
				.map((declaration) => {
					const index = checker.getTypeAtLocation(declaration.name).getNumberIndexType();
					return `${declaration.name.text}: ${index ? checker.typeToString(index) : "(none)"}`;
				}),
		);
		expect(results[1]).toEqual(results[0]);
	});

	test("a function assigned to a method is contextually typed by the method's declaration", () => {
		const source =
			"function apply(object: { SetAttribute(name: string, value: number): number }) {\n  object.SetAttribute = function (this: unknown) {\n    return 1;\n  };\n}\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isFunctionExpression(node)) {
					const contextual = checker.getContextualType(node);
					const declaration = contextual?.getCallSignatures()[0]?.getDeclaration();
					lines.push(`${contextual ? checker.typeToString(contextual) : "(none)"} declared by ${declaration ? ts.SyntaxKind[declaration.kind] : "nothing"}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an iteration result's `done` is the one `true` type, and a variable tuple's positions", () => {
		const source =
			"declare const map: Map<string, number>;\ndeclare const source: [number, ...number[][]];\ndeclare const mid: [number, ...string[], boolean];\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const [map, source, mid] = file("index.ts")
				.statements.filter(ts.isVariableStatement)
				.map((statement) => checker.getTypeAtLocation(statement.declarationList.declarations[0].name));
			const next = checker.getTypeOfPropertyOfType(map.getProperty("entries") ? checker.getTypeOfSymbol(map.getProperty("entries")).getCallSignatures()[0].getReturnType() : map, "next");
			const result = next.getCallSignatures()[0].getReturnType();
			const done = (result.isUnion() ? result.types : [result]).map((part) => checker.getTypeOfPropertyOfType(part, "done") === checker.getTrueType());
			const positions = (tuple) =>
				["0", "1", "2"].map((name) => checker.typeToString(checker.getTypeOfPropertyOfContextualType(tuple, name))).join(" ; ");
			return [`done is true: ${done.join(", ")}`, positions(source), positions(mid), `min ${source.target.minLength} ${mid.target.minLength}`];
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an object literal method's contextual type is the declared member of a possibly undefined target", () => {
		const source =
			"interface Target { SetAttribute(name: string, value: number): number }\ndeclare function apply(object: Target | undefined): void;\napply({\n  SetAttribute(name, value) {\n    return value;\n  },\n});\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isMethodDeclaration(node)) {
					const contextual = checker.getContextualTypeForObjectLiteralElement(node);
					const declaration = contextual?.getCallSignatures()[0]?.getDeclaration();
					lines.push(`${contextual ? checker.typeToString(contextual) : "(none)"} declared by ${declaration ? ts.SyntaxKind[declaration.kind] : "nothing"}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a local export of an import keeps the import", () => {
		const results = bothPrograms(
			{ "index.ts": "import { value, unused } from './lib';\nexport { value };\n", "lib.ts": "export const value = 1;\nexport const unused = 2;\n" },
			["index.ts"],
		).map(({ checker, file }) => {
			const resolver = checker.getEmitResolver();
			const elements = file("index.ts").statements[0].importClause.namedBindings.elements;
			return elements.map((element) => `${element.name.text}: ${resolver.isReferencedAliasDeclaration(element)}`);
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a JSON module's default import aliases its export=", () => {
		const results = bothPrograms(
			{ "index.ts": "import config from './config.json';\nexport const count = config.count;\n", "config.json": '{ "count": 3 }' },
			["index.ts"],
			{ resolveJsonModule: true, esModuleInterop: true },
		).map(({ ts, checker, file }) => {
			const declaration = file("index.ts").statements[0];
			const module = checker.getSymbolAtLocation(declaration.moduleSpecifier);
			const target = checker.getAliasedSymbol(checker.getSymbolAtLocation(declaration.importClause.name));
			return [
				`module ${module ? ts.SyntaxKind[module.valueDeclaration.kind] : "none"}`,
				`target ${target.name} value ${(target.flags & ts.SymbolFlags.Value) !== 0}`,
			];
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("querying a package's declaration of a global interface keeps its other declarations", () => {
		const files = {
			"node_modules/@types-pkg/core/core.d.ts":
				'/// <reference no-default-lib="true"/>\ninterface Array<T> { length: number; [n: number]: T }\ninterface Boolean {}\ninterface Function {}\ninterface IArguments {}\ninterface Number {}\ninterface Object {}\ninterface RegExp {}\ninterface CallableFunction {}\ninterface NewableFunction {}\ninterface Iterable<T> { readonly _nominal_Iterable: T }\n',
			"node_modules/@types-pkg/core/String.d.ts":
				"interface String extends Iterable<string> {\n\treadonly _nominal_String: unique symbol;\n\tsize(this: string): number;\n}\n",
			"node_modules/@types-pkg/core/lua.d.ts": "interface String {\n\tsplit(this: string, separator: string): Array<string>;\n}\n",
			"index.ts": "declare const object: { text: string };\nexport const words = object.text.split(',');\n",
		};
		const roots = ["core.d.ts", "String.d.ts", "lua.d.ts"].map((name) => `node_modules/@types-pkg/core/${name}`);
		const results = bothPrograms(files, [...roots, "index.ts"], { noLib: true, types: [] }).map(({ checker, file }) => {
			for (const statement of file(roots[1]).statements) {
				for (const member of statement.members ?? []) checker.getTypeAtLocation(member);
			}
			const callee = file("index.ts").statements[1].declarationList.declarations[0].initializer.expression;
			const type = checker.getTypeAtLocation(callee);
			return `${checker.typeToString(type)}, symbol ${type.symbol?.name}`;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a class extending a class expression inherits its members", () => {
		const results = bothPrograms(
			{ "index.ts": "class A extends class B {\n\ttoString() { return 'B'; }\n\textra = 1;\n} {}\nexport const a = new A();\n" },
			["index.ts"],
		).map(({ ts, checker, file }) => {
			const declared = checker.getDeclaredTypeOfSymbol(checker.getSymbolAtLocation(file("index.ts").statements[0].name));
			return declared
				.getProperties()
				.map((property) => `${property.name}${property.flags & ts.SymbolFlags.Method ? "()" : ""}`)
				.sort();
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("an `as const` destructuring source keeps its literals, as a pattern's mutable tuple", () => {
		const source =
			"const [b] = [1, 2] as const;\nfor (let [{ value }, , [other]] = [{ value: 0 }, 100, [2]] as const; value < 1; ) {}\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isAsExpression(node) || ts.isArrayLiteralExpression(node) || ts.isObjectLiteralExpression(node) || ts.isBindingElement(node) || ts.isArrayBindingPattern(node))
					lines.push(`${node.getText()}: ${checker.typeToString(checker.getTypeAtLocation(node))}`);
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	/**
	 * What roblox-ts reads off a deferred conditional receiver or parameter
	 * (src/TSTransformer/util/isMethod.ts, `isDefinitelyType`): its flags,
	 * `root`, instantiated `checkType` and `extendsType`, the type nodes of
	 * its branches, and its base constraint.
	 */
	function observeConditionals(ts, checker, sourceFile) {
		const lines = [];
		const text = (type) => {
			if (!type) return "(none)";
			// A union's member order is where each was created (a known difference).
			return type.isUnion() ? type.types.map((member) => checker.typeToString(member)).sort().join(" | ") : checker.typeToString(type);
		};
		const roots = new Set();
		const describe = (label, type) => {
			const conditional = (type.flags & ts.TypeFlags.Conditional) !== 0;
			lines.push(`${label}: ${checker.typeToString(type)} conditional=${conditional} constraint ${text(checker.getBaseConstraintOfType(type))}`);
			if (!conditional) return;
			const { root } = type;
			roots.add(root);
			const alias = type.aliasSymbol ? `${type.aliasSymbol.name}<${type.aliasTypeArguments.map(text).join(", ")}>` : "-";
			const infer = (root.inferTypeParameters ?? []).map(text).join(", ");
			lines.push(`  alias ${alias} root ${root.aliasSymbol?.name ?? "-"} distributive=${root.isDistributive} infer [${infer}]`);
			lines.push(`  root ${text(root.checkType)} extends ${text(root.extendsType)}; ${text(type.checkType)} extends ${text(type.extendsType)}`);
			const trueType = checker.getTypeFromTypeNode(root.node.trueType);
			const falseType = checker.getTypeFromTypeNode(root.node.falseType);
			lines.push(`  true ${text(trueType)} never=${(trueType.flags & ts.TypeFlags.Never) !== 0} false is root check ${falseType === root.checkType}`);
			if (checker.isTupleType(root.checkType)) lines.push(`  false is check element ${falseType === checker.getTypeArguments(root.checkType)[0]}`);
			lines.push(`  void extends ${checker.isTypeAssignableTo(checker.getVoidType(), type.extendsType)}`);
		};
		ts.forEachChild(sourceFile, function visit(node) {
			if (ts.isParameter(node) && ts.isIdentifier(node.name) && node.type) {
				describe(`${node.name.text}@${node.getStart()}`, checker.getTypeAtLocation(node.name));
				if (node.name.text === "this") {
					const signature = checker.getTypeAtLocation(node.parent).getCallSignatures()[0];
					describe("  thisParameter", checker.getTypeOfSymbolAtLocation(signature.thisParameter, node.parent));
				}
			}
			if (ts.isSpreadElement(node)) describe(`...${node.expression.getText()}`, checker.getTypeAtLocation(node.expression));
			ts.forEachChild(node, visit);
		});
		lines.push(`roots ${roots.size}`);
		return lines;
	}

	test("a deferred conditional's root, instantiated check and extends types, and base constraint", () => {
		const source =
			"type TypeGuard<T> = (value: unknown) => value is T;\ntype Args<T> = T extends [TypeGuard<infer A>] ? [A] : T extends [TypeGuard<infer A>, TypeGuard<infer B>] ? [A, B] : Array<unknown>;\nfunction spread<C extends Array<unknown>>(args: Args<C>) { return [...args]; }\nfunction make<T>() {\n  return {\n    a(this: Exclude<T, undefined>, value: number) { return value; },\n    b(this: T extends string ? never : void, value: number) { return value; },\n    c(this: [T] extends [void] ? never : T, value: number) { return value; },\n    d(this: Exclude<T, void>, value: number) { return value; },\n  };\n}\nfunction pick<T extends number | string>(value: T extends number ? 'number' : 'string') { return value; }\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => observeConditionals(ts, checker, file("index.ts")));
		expect(results[1]).toEqual(results[0]);
	});

	test("a `this` conditional over the function's own type parameters is deferred", () => {
		const source =
			"type Ex<T> = T extends void ? never : T;\nfunction viaAlias<T>(this: Ex<T>, value: number) { return value; }\nfunction inline<T>(this: T extends string ? void : {}, value: number) { return value + 1; }\nfunction tupled<T>(this: [T] extends T ? never : T) {}\nfunction optionalTarget<T>(this: [T] extends [{}, T?] ? never : T) {}\ninline<string>(41);\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isFunctionDeclaration(node) || ts.isCallExpression(node)) {
					const type = checker.getTypeAtLocation(ts.isCallExpression(node) ? node.expression : node);
					for (const signature of type.getCallSignatures()) {
						const thisType = checker.getTypeOfSymbolAtLocation(signature.thisParameter, node);
						const constraint = checker.getBaseConstraintOfType(thisType);
						const conditional = (thisType.flags & ts.TypeFlags.Conditional) !== 0;
						lines.push(`${(node.name ?? node.expression).getText()}: conditional=${conditional} constraint ${constraint ? checker.typeToString(constraint) : "(none)"}`);
					}
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a signature's `this` instantiated by a generic alias inside a generic function is that function's type parameter", () => {
		const source =
			"type Callable<T> = (this: T, value: number) => number;\nfunction make<T extends {}>() {\n  const callback: Callable<T> = (value) => value + 1;\n  return { callback };\n}\nconst mul = function <T extends { n: number }>(this: T, n: number) { return this; };\nexport {};\n";
		const results = bothPrograms({ "index.ts": source }, ["index.ts"]).map(({ ts, checker, file }) => {
			const lines = [];
			ts.forEachChild(file("index.ts"), function visit(node) {
				if (ts.isVariableDeclaration(node)) {
					const signature = checker.getTypeAtLocation(node.name).getCallSignatures()[0];
					const thisType = checker.getTypeOfSymbolAtLocation(signature.thisParameter, node);
					const constraint = checker.getBaseConstraintOfType(thisType);
					lines.push(`${node.name.text}: this ${checker.typeToString(thisType)} constraint ${constraint ? checker.typeToString(constraint) : "(none)"}`);
				}
				ts.forEachChild(node, visit);
			});
			return lines;
		});
		expect(results[1]).toEqual(results[0]);
	});

	test("a generic member a noLib program's own Array declares is instantiated for the call", () => {
		const files = {
			"lib.d.ts":
				"interface Boolean {}\ninterface Function {}\ninterface IArguments {}\ninterface Number {}\ninterface Object {}\ninterface RegExp {}\ninterface String {}\ninterface CallableFunction {}\ninterface NewableFunction {}\ntype NonNullable<T> = T & {};\ninterface ReadonlyArray<T> {\n\treadonly [n: number]: T;\n\tmapFiltered<U>(this: ReadonlyArray<{}>, callback: (value: T, index: number) => U): Array<NonNullable<U>>;\n}\ninterface Array<T> extends ReadonlyArray<T> {\n\tlength: number;\n}\n",
			"index.ts": "declare const values: Array<number>;\nexport const halves = values.mapFiltered((value) => (value > 0 ? value / 2 : undefined));\n",
		};
		const results = bothPrograms(files, ["lib.d.ts", "index.ts"], { noLib: true, types: [] }).map(({ checker, file }) => {
			const declaration = file("index.ts").statements[1].declarationList.declarations[0];
			return `${checker.typeToString(checker.getTypeAtLocation(declaration.initializer))}`;
		});
		expect(results[1]).toEqual(results[0]);
	});
});

/**
 * What roblox-ts reads off a call to choose `obj:m()` or `obj.m()`
 * (src/TSTransformer/util/isMethod.ts): the callee type's symbol, and each
 * call signature's `this` parameter and declaration.
 */
function observeReceivers(ts, program, fileName) {
	const checker = program.getTypeChecker();
	const sourceFile = program.getSourceFile(fileName);
	const lines = [];
	const visit = (node) => {
		if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
			lines.push(`${node.name.text}: ${checker.typeToString(checker.getTypeAtLocation(node.name))}`);
		}
		if (ts.isCallExpression(node) && (ts.isPropertyAccessExpression(node.expression) || ts.isElementAccessExpression(node.expression))) {
			const callee = node.expression;
			const type = checker.getNonOptionalType(checker.getTypeAtLocation(callee));
			const signatures = type.getCallSignatures().map((signature) => {
				const thisType = signature.thisParameter && checker.getTypeOfSymbolAtLocation(signature.thisParameter, callee);
				const declaration = signature.declaration ? ts.SyntaxKind[signature.declaration.kind] : "-";
				return `this ${thisType ? checker.typeToString(thisType) : "-"} declaration ${declaration}`;
			});
			lines.push(`${callee.getText(sourceFile)}: symbol ${type.symbol?.name ?? "-"} [${signatures.join("; ")}]`);
		}
		ts.forEachChild(node, visit);
	};
	visit(sourceFile);
	return lines;
}

describe("project references agree with typescript@6", () => {
	test("a referenced project's sources are its output declarations, and an unbuilt one's import is TS6305", () => {
		const composite = { composite: true, declaration: true, strict: true, target: "esnext", module: "esnext", moduleResolution: "bundler", rootDir: "src", outDir: "out" };
		const dir = write({
			"shared/tsconfig.json": JSON.stringify({ compilerOptions: composite, include: ["src"] }),
			"shared/src/index.ts": "export const value: number = 1;\n",
			"shared/src/unbuilt.ts": "export const other: number = 1;\n",
			"shared/out/index.d.ts": "export declare const value: string;\n",
			"app/tsconfig.json": JSON.stringify({
				compilerOptions: { strict: true, target: "esnext", module: "esnext", moduleResolution: "bundler", noEmit: true },
				include: ["src"],
				references: [{ path: "../shared" }],
			}),
			"app/src/main.ts": 'import { value } from "../../shared/src/index";\nimport { other } from "../../shared/src/unbuilt";\nconst read: number = value;\n',
		});
		const relative = (file) => path.relative(dir, file).replaceAll("\\", "/");
		const results = [typescript, surge].map((ts) => {
			const parsed = ts.getParsedCommandLineOfConfigFile(path.join(dir, "app/tsconfig.json"), {}, { ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} });
			const program = ts.createProgram({ rootNames: parsed.fileNames, options: parsed.options, projectReferences: parsed.projectReferences });
			const main = program.getSourceFile(path.join(dir, "app/src/main.ts"));
			const checker = program.getTypeChecker();
			const modules = main.statements
				.filter(ts.isImportDeclaration)
				.map((statement) => checker.getSymbolAtLocation(statement.moduleSpecifier)?.declarations?.[0]?.fileName);
			return {
				// surge orders a program's files by load, not by dependency.
				files: program
					.getSourceFiles()
					.map((file) => relative(file.fileName))
					.filter((name) => !name.includes("node_modules") && !name.startsWith(".."))
					.sort(),
				references: program.getProjectReferences()?.map((reference) => relative(reference.path)),
				resolved: program.getResolvedProjectReferences()?.map((reference) => relative(reference.sourceFile.fileName)),
				resolvedFiles: program.getResolvedProjectReferences()?.map((reference) => reference.commandLine.fileNames.map(relative).sort()),
				diagnostics: ts.getPreEmitDiagnostics(program, main).map((diagnostic) => `${diagnostic.code}:${diagnostic.start}`),
				modules: modules.map((file) => file && relative(file)),
			};
		});
		expect(results[1]).toEqual(results[0]);
	});
});

describe("call receivers agree with typescript@6", () => {
	const compareReceivers = (text) => {
		const dir = write({ "index.ts": text });
		const fileName = path.join(dir, "index.ts");
		const expected = observeReceivers(typescript, typescript.createProgram([fileName], options), fileName);
		const actual = observeReceivers(surge, surge.createProgram([fileName], options), fileName);
		expect(actual.join("\n")).toBe(expected.join("\n"));
	};

	test("a function read through an object property, and a function expression's `this` parameter", () => {
		compareReceivers(
			"type Receiver = { value: number };\nfunction callback(this: void, value: number) { return value; }\nfunction method(this: Receiver, value: number) { return this.value + value; }\nconst callbackExpression = function (this: void, value: number) { return value; };\nconst methodExpression = function (this: Receiver, value: number) { return this.value + value; };\nconst object = { value: 10, callback, method, callbackExpression, methodExpression, renamed: method };\nobject.callback(1);\nobject.method(1);\nobject.callbackExpression(1);\nobject.methodExpression(1);\nobject.renamed(1);\nfunction make<T extends {}>() { return { method(this: T, value: number) { return value; } }; }\nconst made = make<{}>();\nmade.method(1);\nexport {};\n",
		);
	});

	test("a member called through an optional chain is the member's own type", () => {
		compareReceivers(
			"declare const values: number[] | undefined;\nvalues?.map((value) => value);\ninterface Part { SetAttribute(this: Part, name: string, value: number): void }\ndeclare const part: Part | undefined;\npart?.SetAttribute('a', 1);\npart?.['SetAttribute']('a', 1);\ndeclare const holder: { part?: Part } | undefined;\nholder?.part?.SetAttribute('a', 1);\nexport {};\n",
		);
	});

	test("a class expression's static members", () => {
		compareReceivers(
			"const value = class { static $() { return 1; } }.$();\nconst parenthesized = (class { static $() { return 1; } })['$']();\nconst instance = new (class { $() { return 1; } })().$();\nexport {};\n",
		);
	});

	test("static and instance members read through `super`, `this` and the class", () => {
		compareReceivers(
			"type Callable<T> = (this: T, value: number) => number;\nclass Base {\n  static callback: Callable<void> = (value) => value;\n  static method(value: number) { return value; }\n  method(value: string) { return value; }\n}\nclass Derived extends Base {\n  static check() { super.callback(1); super['callback'](1); super.method(1); super['method'](1); Base['method'](1); }\n  run() { super.method(''); super['method'](''); this.method(''); this['method'](''); }\n}\nexport {};\n",
		);
	});
});

// Each of these is a difference surge-ts has today, reduced to one construct
// and listed in docs/COMPILER_API.md. `test.fails` passes while the outputs
// differ, so fixing one fails its test here until it moves into the suite above.
describe("known differences from typescript@6", () => {
	test.fails("an array method read off a union of tuples is a union of each member's", () => {
		const dir = write({ "index.ts": "declare const tuples: [number, number, ...string[]] | [];\ntuples.pop();\nexport {};\n" });
		const [expected, actual] = [typescript, surge].map((ts) => {
			const program = ts.createProgram([path.join(dir, "index.ts")], options);
			const checker = program.getTypeChecker();
			const call = program.getSourceFile(path.join(dir, "index.ts")).statements[1].expression;
			const type = checker.getTypeAtLocation(call.expression);
			return type.isUnion() ? type.types.length : 1;
		});
		expect(actual).toEqual(expected);
	});

	test.fails("a tuple type alias keeps its name", () => {
		compare({ "index.ts": "type Pair = [string, number];\ndeclare const pair: Pair;\nexport {};\n" });
	});

});
