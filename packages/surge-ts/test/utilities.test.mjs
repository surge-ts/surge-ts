// TypeScript's internal syntax, scanner, path and option helpers, compared
// with typescript@6.0.3's own (its runtime exports its internals) on every
// node of sources written to exercise them.
import typescript from "typescript-6";
import { describe, expect, test } from "vitest";
import surge from "../index.mjs";

const SOURCE = `
// leading comment
/** doc */ export abstract class Base<T> extends Object implements Iterable<T> {
	static count = 0; // trailing comment
	protected abstract readonly name: string;
	accessor value = 1;
	constructor(private readonly id: number, public label?: string) { super(); }
	@decorate method(this: Base<T>, ...rest: number[]) { return this.id + super.valueOf(); }
	get size() { return 1; }
	static { Base.count++; }
	[Symbol.iterator]: any;
	["computed"]() {}
	[-1]: number;
	declare override x: number;
}
declare function decorate(...args: unknown[]): any;
export interface Shape extends Base<number>, Iterable<string> { readonly kind: "shape" }
namespace Types { export interface I {} export type T = number; }
namespace Values { export const v = 1; }
namespace ConstOnly { export const enum E { A } }
namespace Reexport { interface Hidden {} export { Hidden }; }
namespace Outer.Inner { export let deep = 1; }
const enum Flag { On = 1 }
export const { a, b: [c, ...d] } = { a: 1, b: [2, 3] } as const;
let x = 0, y: number | undefined;
x = 1; x += 2; x ||= 3; y ??= 4; x++; --x;
[x, y] = [y ?? 0, x];
({ x, y = 1 } = { x: 1, y: 2 });
for (x of [1]) {}
for (const k in {}) {}
label: for (;;) { break label; }
do {} while (false);
const o = { x } as { x: number } | undefined;
o?.x; o!.x;
Base.prototype.method;
Base["prototype"];
(async () => { await import("./m"); })();
function f(this: unknown) { return function inner() { return () => this; }; }
const t = \`a\${x}b\`, n = -1, s = 'str', template = \`plain\`;
import type { Kind } from "./kinds";
import * as ns from "./ns";
export type { Kind as Kind2 } from "./kinds";
export * as all from "./ns";
export { ns };
`;

const nodeHelpers = [
	"getSyntacticModifierFlags",
	"getEffectiveModifierFlags",
	"getCombinedModifierFlags",
	"getCombinedNodeFlags",
	"canHaveModifiers",
	"canHaveDecorators",
	"hasStaticModifier",
	"hasAbstractModifier",
	"hasAccessorModifier",
	"hasDecorators",
	"getModifiers",
	"getDecorators",
	"isFunctionLike",
	"isFunctionLikeDeclaration",
	"isFunctionBody",
	"isOptionalChain",
	"isStringLiteralLike",
	"isStringOrNumericLiteralLike",
	"isSignedNumericLiteral",
	"isAutoAccessorPropertyDeclaration",
	"isTypeOnlyExportDeclaration",
	"isAnyImportSyntax",
	"isImportCall",
	"isSuperCall",
	"isSuperProperty",
	"isThisIdentifier",
	"isEntityNameExpression",
	"isPrototypeAccess",
	"isAssignmentExpression",
	"isLogicalOrCoalescingAssignmentExpression",
	"isAssignmentTarget",
	"getAssignmentTargetKind",
	"isWriteAccess",
	"isWriteOnlyAccess",
	"getAllSuperTypeNodes",
	"getEffectiveBaseTypeNode",
	"getEffectiveImplementsTypeNodes",
	"getInterfaceBaseTypeNodes",
];

/** A helper's result, comparable across implementations. */
function describe_(ts, value) {
	if (value === undefined || value === null || typeof value !== "object") return value;
	if (Array.isArray(value)) return value.map((item) => describe_(ts, item));
	if (typeof value.kind === "number" && typeof value.pos === "number") return `${ts.SyntaxKind[value.kind]}@${value.pos}`;
	return JSON.stringify(value);
}

function observe(ts, fileName, text) {
	const sourceFile = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true);
	const lines = [];
	const call = (name, ...args) => {
		try {
			return describe_(ts, ts[name](...args));
		} catch (error) {
			return `throws ${error.constructor.name}`;
		}
	};
	ts.forEachChild(sourceFile, function visit(node) {
		const where = `${ts.SyntaxKind[node.kind]}@${node.pos}`;
		for (const name of nodeHelpers) lines.push(`${where} ${name} ${JSON.stringify(call(name, node))}`);
		lines.push(`${where} isIterationStatement ${call("isIterationStatement", node, true)}`);
		lines.push(`${where} getThisContainer ${call("getThisContainer", node, true, false)}`);
		lines.push(`${where} getThisContainer(no arrows) ${call("getThisContainer", node, false, true)}`);
		if (node.parent) lines.push(`${where} isParameterPropertyDeclaration ${call("isParameterPropertyDeclaration", node, node.parent)}`);
		if (ts.isModuleDeclaration(node)) {
			lines.push(`${where} isInstantiatedModule ${call("isInstantiatedModule", node, false)} ${call("isInstantiatedModule", node, true)}`);
		}
		if (node.name && ts.isPropertyName(node.name)) {
			lines.push(`${where} getPropertyNameForPropertyNameNode ${call("getPropertyNameForPropertyNameNode", node.name)}`);
		}
		lines.push(`${where} comments ${JSON.stringify(ts.getLeadingCommentRanges(text, node.pos))} ${JSON.stringify(ts.getTrailingCommentRanges(text, node.end))}`);
		ts.forEachChild(node, visit);
	});
	return lines;
}

describe("TypeScript's internal helpers agree with typescript@6", () => {
	test("syntax and scanner helpers over every node", () => {
		expect(observe(surge, "/src/index.ts", SOURCE).join("\n")).toBe(observe(typescript, "/src/index.ts", SOURCE).join("\n"));
	});

	test("token children: getChildren, getFirstToken, getLastToken", () => {
		// surge-ts's tree has no JSDoc nodes, which TypeScript lists first.
		const isNotJSDoc = (node) => node.kind < typescript.SyntaxKind.FirstJSDocNode || node.kind > typescript.SyntaxKind.LastJSDocNode;
		const tokens = (ts, fileName, text) => {
			const sourceFile = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true);
			const describeTree = (node) =>
				`${ts.SyntaxKind[node.kind]}@${node.pos}-${node.end}` +
				(node.kind >= ts.SyntaxKind.FirstNode ? `(${node.getChildren(sourceFile).filter(isNotJSDoc).map(describeTree).join(" ")})` : "");
			const lines = [describeTree(sourceFile)];
			ts.forEachChild(sourceFile, function visit(node) {
				const first = node.getFirstToken(sourceFile);
				const last = node.getLastToken(sourceFile);
				lines.push(`${ts.SyntaxKind[node.kind]}@${node.pos} ${node.getChildren(sourceFile).filter(isNotJSDoc).length} ${first && ts.SyntaxKind[first.kind]}@${first?.pos} ${last && ts.SyntaxKind[last.kind]}@${last?.pos} ${last?.getText(sourceFile)}`);
				ts.forEachChild(node, visit);
			});
			return lines.join("\n");
		};
		const view = "const e = <div a='1' {...rest}>{x} text <b /></div>;\nconst f = <></>;\nexport {};\n";
		expect(tokens(surge, "/src/index.ts", SOURCE)).toBe(tokens(typescript, "/src/index.ts", SOURCE));
		expect(tokens(surge, "/src/view.tsx", view)).toBe(tokens(typescript, "/src/view.tsx", view));
	});

	test("JSX namespaced names", () => {
		const text = "const e = <a:b c:d='1' />;\nexport {};\n";
		const names = (ts) => {
			const out = [];
			const sourceFile = ts.createSourceFile("/src/view.tsx", text, ts.ScriptTarget.Latest, true);
			ts.forEachChild(sourceFile, function visit(node) {
				if (ts.isJsxNamespacedName(node)) out.push(ts.getTextOfJsxNamespacedName(node), ts.getEscapedTextOfJsxNamespacedName(node));
				ts.forEachChild(node, visit);
			});
			return out;
		};
		expect(names(surge)).toEqual(names(typescript));
	});

	test("path helpers", () => {
		const samples = [
			"",
			".",
			"./a/../b",
			"a/./b//c/",
			"../../x",
			"/",
			"/a/b/../../..",
			"C:",
			"C:\\Users\\me\\..\\project\\src\\",
			"c:/a/./b",
			"\\\\server\\share\\dir",
			"file:///C:/a/b",
			"file://localhost/c%3A/x",
			"https://example.com/a/../b",
			"node_modules/@types/x/index.d.ts",
			"lib.dom.d.ts",
			"styles.d.css.ts",
			"a.json",
		];
		const helpers = [
			"normalizeSlashes",
			"normalizePath",
			"getRootLength",
			"isRootedDiskPath",
			"hasTrailingDirectorySeparator",
			"ensureTrailingDirectorySeparator",
			"removeTrailingDirectorySeparator",
			"getDirectoryPath",
			"getBaseFileName",
			"getPathComponents",
			"isDeclarationFileName",
			"removeFileExtension",
			"isInsideNodeModules",
		];
		const run = (ts) =>
			samples.flatMap((sample) => [
				...helpers.map((name) => `${name}(${sample}) = ${JSON.stringify(ts[name](sample))}`),
				`getNormalizedAbsolutePath = ${ts.getNormalizedAbsolutePath(sample, "/root/cwd")}`,
				`toPath = ${ts.toPath(sample, "/Root/Cwd", ts.createGetCanonicalFileName(false))}`,
				`combinePaths = ${ts.combinePaths("/base", sample, "tail")}`,
				`resolvePath = ${ts.resolvePath("/base/x", sample)}`,
				`reducePathComponents = ${ts.reducePathComponents(ts.getPathComponents(sample, "/cwd"))}`,
				`getRelativePathFromDirectory = ${ts.isRootedDiskPath(sample) ? ts.getRelativePathFromDirectory("/a/b", sample, true) : ""}`,
			]);
		expect(run(surge).join("\n")).toBe(run(typescript).join("\n"));
	});

	test("compiler option helpers", () => {
		const options = [
			{},
			{ declaration: true },
			{ composite: true, configFilePath: "/p/tsconfig.json" },
			{ incremental: true, configFilePath: "/p/tsconfig.json", outDir: "/p/out" },
			{ incremental: true, configFilePath: "/p/src/tsconfig.json", outDir: "/p/out", rootDir: "/p" },
			{ incremental: true, outFile: "/p/out.js" },
			{ incremental: true, tsBuildInfoFile: "/p/x.tsbuildinfo" },
		];
		const run = (ts) =>
			options.map((option) => [ts.getEmitDeclarations(option), ts.isIncrementalCompilation(option), ts.getTsBuildInfoEmitOutputFilePath(option)]);
		expect(run(surge)).toEqual(run(typescript));
		const configs = [
			{ fileNames: [], raw: { references: [] } },
			{ fileNames: ["a.ts"], raw: { references: [] } },
			{ fileNames: [], raw: {} },
		];
		expect(configs.map(surge.isSolutionConfig)).toEqual(configs.map(typescript.isSolutionConfig));
	});

	test("diagnostic messages", () => {
		expect(Object.keys(surge.Diagnostics)).toEqual(Object.keys(typescript.Diagnostics));
		for (const name of ["Cannot_find_name_0", "Option_0_can_only_be_specified_in_tsconfig_json_file_or_set_to_null_on_command_line"]) {
			expect(surge.Diagnostics[name]).toEqual(JSON.parse(JSON.stringify(typescript.Diagnostics[name])));
		}
		const diagnostic = (ts) => ts.createCompilerDiagnostic(ts.Diagnostics.Cannot_find_name_0, "x");
		expect(diagnostic(surge)).toEqual(diagnostic(typescript));
	});
});
