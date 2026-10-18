// The TypeScript-shaped facade over programs on disk and in memory.
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, test } from "vitest";
import ts from "../index.mjs";

function project(files) {
	const dir = realpathSync(mkdtempSync(path.join(tmpdir(), "surge-ts-api-")));
	for (const [name, text] of Object.entries(files)) {
		const file = path.join(dir, name);
		mkdirSync(path.dirname(file), { recursive: true });
		writeFileSync(file, text);
	}
	return dir;
}

const options = {
	strict: true,
	noEmit: true,
	target: ts.ScriptTarget.ESNext,
	module: ts.ModuleKind.ESNext,
	moduleResolution: ts.ModuleResolutionKind.Bundler,
};

function nodesOf(sourceFile) {
	const nodes = [];
	ts.forEachChild(sourceFile, function visit(node) {
		nodes.push(node);
		ts.forEachChild(node, visit);
	});
	return nodes;
}

function findIdentifier(sourceFile, name, occurrence = 0) {
	const found = nodesOf(sourceFile).filter((node) => ts.isIdentifier(node) && node.text === name);
	expect(found[occurrence], `identifier ${name} #${occurrence}`).toBeDefined();
	return found[occurrence];
}

describe("enums", () => {
	test("carry TypeScript's values and reverse mappings", () => {
		expect(ts.SyntaxKind.Identifier).toBe(80);
		expect(ts.SyntaxKind[80]).toBe("Identifier");
		expect(ts.SyntaxKind.FirstStatement).toBe(ts.SyntaxKind.VariableStatement);
		expect(ts.SignatureKind.Construct).toBe(1);
		expect(ts.DiagnosticCategory[ts.DiagnosticCategory.Error]).toBe("Error");
		expect(ts.ScriptTarget.ESNext).toBe(99);
		expect(ts.TypeFlags.Union).toBe(1 << 27);
		expect(ts.Extension.Dts).toBe(".d.ts");
	});
});

describe("programs", () => {
	test("the first-milestone script: walk nodes, symbols, types, diagnostics", () => {
		const dir = project({
			"src/index.ts":
				"import { greet } from './lib';\nconst who = { name: 'Ada' };\nconst message = greet(who.name);\nconst wrong: number = message;\n",
			"src/lib.ts": "export function greet(name: string): string { return `Hello, ${name}`; }\n",
		});
		const program = ts.createProgram([path.join(dir, "src/index.ts")], options);
		const checker = program.getTypeChecker();
		const seen = [];
		for (const sourceFile of program.getSourceFiles()) {
			if (sourceFile.isDeclarationFile) continue;
			ts.forEachChild(sourceFile, function visit(node) {
				const symbol = checker.getSymbolAtLocation(node);
				if (symbol) seen.push(`${symbol.getName()}: ${checker.typeToString(checker.getTypeOfSymbolAtLocation(symbol, node))}`);
				ts.forEachChild(node, visit);
			});
		}
		expect(seen).toContain("message: string");
		expect(seen).toContain("greet: (name: string) => string");
		expect(seen).toContain("name: string");
		const diagnostics = ts.getPreEmitDiagnostics(program);
		expect(diagnostics.map((diagnostic) => [path.basename(diagnostic.file.fileName), diagnostic.code])).toEqual([["index.ts", 2322]]);
		expect(ts.flattenDiagnosticMessageText(diagnostics[0].messageText, "\n")).toBe("Type 'string' is not assignable to type 'number'.");
	});

	test("the object form and the program's accessors", () => {
		const dir = project({ "a.ts": "export const a = 1;\n" });
		const program = ts.createProgram({ rootNames: [path.join(dir, "a.ts")], options });
		expect(program.getRootFileNames()).toEqual([path.join(dir, "a.ts")]);
		expect(program.getCompilerOptions().strict).toBe(true);
		const sourceFile = program.getSourceFile(path.join(dir, "a.ts"));
		expect(sourceFile.kind).toBe(ts.SyntaxKind.SourceFile);
		const [firstFile] = program.getSourceFiles();
		expect(firstFile.isDeclarationFile, "default libraries come first").toBe(true);
		expect(program.isSourceFileDefaultLibrary(firstFile)).toBe(true);
		expect(program.isSourceFileFromExternalLibrary(sourceFile)).toBe(false);
		expect(program.getSemanticDiagnostics(sourceFile)).toEqual([]);
		expect(() => program.emit()).toThrow(/not supported/);
	});

	test("handles from another program are rejected", () => {
		const dir = project({ "index.ts": "const x = 1;\n" });
		const first = ts.createProgram([path.join(dir, "index.ts")], options);
		const second = ts.createProgram([path.join(dir, "index.ts")], options);
		const node = findIdentifier(first.getSourceFile(path.join(dir, "index.ts")), "x");
		expect(() => second.getTypeChecker().getTypeAtLocation(node)).toThrow(TypeError);
		const type = first.getTypeChecker().getTypeAtLocation(node);
		expect(() => second.getTypeChecker().typeToString(type)).toThrow(TypeError);
	});

	test("a CompilerHost can serve files from memory", () => {
		const files = new Map([
			["/memory/main.ts", "import { two } from './two';\nconst s: string = two;\n"],
			["/memory/two.ts", "export const two = 2;\n"],
		]);
		const host = {
			...ts.createCompilerHost(options),
			readFile: (fileName) => files.get(fileName),
			fileExists: (fileName) => files.has(fileName),
			directoryExists: (directory) => [...files.keys()].some((file) => file.startsWith(`${directory}/`)),
			getCurrentDirectory: () => "/memory",
			getDirectories: () => [],
			realpath: (fileName) => fileName,
		};
		const program = ts.createProgram(["main.ts"], options, host);
		const ownFiles = program
			.getSourceFiles()
			.filter((sourceFile) => !sourceFile.isDeclarationFile)
			.map((sourceFile) => sourceFile.fileName);
		expect(ownFiles).toEqual(["/memory/main.ts", "/memory/two.ts"]);
		expect(ts.getPreEmitDiagnostics(program).map((diagnostic) => diagnostic.code)).toEqual([2322]);
	});
});

describe("nodes", () => {
	test("kinds, positions, parents, text, named children and identity", () => {
		const dir = project({ "index.ts": "function add(a: number, b = 2) {\n  return a + b;\n}\nadd(1);\n" });
		const program = ts.createProgram([path.join(dir, "index.ts")], options);
		const sourceFile = program.getSourceFile(path.join(dir, "index.ts"));
		const [declaration, statement] = sourceFile.statements;
		expect(sourceFile.statements.pos).toBe(0);
		expect(sourceFile.statements.end).toBe(sourceFile.endOfFileToken.pos);
		expect(ts.isFunctionDeclaration(declaration)).toBe(true);
		expect(declaration.name.text).toBe("add");
		expect(declaration.name.escapedText).toBe("add");
		expect(declaration.parameters).toHaveLength(2);
		expect(declaration.parameters[1].initializer.getText()).toBe("2");
		expect(declaration.parameters.hasTrailingComma).toBe(false);
		expect(ts.isExpressionStatement(statement)).toBe(true);
		const call = statement.expression;
		expect(ts.isCallExpression(call)).toBe(true);
		expect(call.getText()).toBe("add(1)");
		expect(call.getStart()).toBe(sourceFile.text.indexOf("add(1)"));
		expect(call.parent).toBe(statement);
		expect(call.getSourceFile()).toBe(sourceFile);
		expect(call.arguments[0].parent, "one object per node").toBe(call);
		const binary = nodesOf(sourceFile).find(ts.isBinaryExpression);
		expect(binary.operatorToken.kind).toBe(ts.SyntaxKind.PlusToken);
		expect(ts.SyntaxKind[binary.left.kind]).toBe("Identifier");
		const lists = [];
		ts.forEachChild(
			declaration,
			() => undefined,
			(nodes) => void lists.push(nodes.length),
		);
		expect(lists, "cbNodes receives the parameter list").toEqual([2]);
		const kinds = (nodes) => nodes.map((child) => ts.SyntaxKind[child.kind]);
		expect(kinds(call.getChildren())).toEqual(["Identifier", "OpenParenToken", "SyntaxList", "CloseParenToken"]);
		expect(call.getChildAt(2).getChildren().map((child) => child.kind)).toEqual([ts.SyntaxKind.NumericLiteral]);
		expect(call.getLastToken().getText()).toBe(")");
		expect(call.arguments[0].getChildren(), "a token has no children").toEqual([]);
	});

	test("positions are UTF-16 offsets and lines follow TypeScript", () => {
		const dir = project({ "index.ts": 'const s = "😀";\r\nconst n: number = s;\n' });
		const program = ts.createProgram([path.join(dir, "index.ts")], options);
		const sourceFile = program.getSourceFile(path.join(dir, "index.ts"));
		const n = findIdentifier(sourceFile, "n");
		expect(n.getStart()).toBe(sourceFile.text.indexOf("n:"));
		expect(sourceFile.getLineAndCharacterOfPosition(n.getStart())).toEqual({ line: 1, character: 6 });
		expect(sourceFile.getPositionOfLineAndCharacter(1, 6)).toBe(n.getStart());
		expect(ts.getLineAndCharacterOfPosition(sourceFile, 0)).toEqual({ line: 0, character: 0 });
		const [diagnostic] = program.getSemanticDiagnostics(sourceFile);
		expect(diagnostic.start).toBe(n.getStart());
		expect(sourceFile.text.substr(diagnostic.start, diagnostic.length)).toBe("n");
	});

	test("createSourceFile parses text on its own", () => {
		const sourceFile = ts.createSourceFile("/x.ts", "const answer = 42;\n", ts.ScriptTarget.Latest, true);
		const [statement] = sourceFile.statements;
		expect(ts.isVariableStatement(statement)).toBe(true);
		const [declaration] = statement.declarationList.declarations;
		expect(declaration.name.getText()).toBe("answer");
		expect(declaration.initializer.text).toBe("42");
		expect(statement.declarationList.flags & ts.NodeFlags.Const).toBe(ts.NodeFlags.Const);
	});
});

describe("checker", () => {
	test("types, symbols and signatures keep their identity and answer queries", () => {
		const dir = project({
			"index.ts":
				"interface Point { x: number; y?: number }\nfunction measure(p: Point): number { return p.x; }\nconst origin: Point = { x: 0 };\nconst d = measure(origin);\n",
		});
		const program = ts.createProgram([path.join(dir, "index.ts")], options);
		const checker = program.getTypeChecker();
		const sourceFile = program.getSourceFile(path.join(dir, "index.ts"));
		const origin = findIdentifier(sourceFile, "origin");
		expect(checker.getTypeAtLocation(origin)).toBe(checker.getTypeAtLocation(origin));
		const pointSymbol = checker.getSymbolAtLocation(findIdentifier(sourceFile, "Point"));
		expect(pointSymbol.flags & ts.SymbolFlags.Interface).toBe(ts.SymbolFlags.Interface);
		expect(pointSymbol.declarations).toHaveLength(1);
		expect(checker.getSymbolAtLocation(findIdentifier(sourceFile, "Point", 1))).toBe(pointSymbol);
		const point = checker.getDeclaredTypeOfSymbol(pointSymbol);
		expect(checker.typeToString(point)).toBe("Point");
		expect(point.getProperties().map((property) => property.getName())).toEqual(["x", "y"]);
		expect(checker.typeToString(checker.getTypeOfSymbol(point.getProperty("y")))).toBe("number | undefined");
		expect(point.getProperty("x").valueDeclaration.kind).toBe(ts.SyntaxKind.PropertySignature);
		const measure = checker.getTypeOfSymbol(checker.getSymbolAtLocation(findIdentifier(sourceFile, "measure")));
		const [signature] = measure.getCallSignatures();
		expect(checker.typeToString(signature.getReturnType())).toBe("number");
		expect(checker.signatureToString(signature)).toBe("(p: Point): number");
		expect(signature.getParameters().map((parameter) => parameter.name)).toEqual(["p"]);
		expect(signature.getDeclaration().kind).toBe(ts.SyntaxKind.FunctionDeclaration);
		expect(checker.isTypeAssignableTo(checker.getNumberType(), checker.getNumberType())).toBe(true);
		expect(checker.isTypeAssignableTo(checker.getStringType(), checker.getNumberType())).toBe(false);
		expect(checker.getNumberType().flags).toBe(ts.TypeFlags.Number);
		expect(checker.typeToString(checker.getTypeAtLocation(findIdentifier(sourceFile, "d")))).toBe("number");
	});

	test("literal widening, constraints, and what is not answered yet", () => {
		const dir = project({
			"index.ts":
				"interface Base { id: number }\ninterface Named extends Base { name: string }\ntype Mode = 'light' | 'dark';\ndeclare const mode: Mode;\ntype Box = { value: number };\ndeclare const box: Box;\nconst one = 1;\nfunction first<T extends string>(items: T[]): T { return items[0]; }\n",
		});
		const program = ts.createProgram([path.join(dir, "index.ts")], options);
		const checker = program.getTypeChecker();
		const sourceFile = program.getSourceFile(path.join(dir, "index.ts"));
		const typeOf = (name) => checker.getTypeOfSymbol(checker.getSymbolAtLocation(findIdentifier(sourceFile, name)));
		expect(checker.typeToString(checker.getBaseTypeOfLiteralType(typeOf("mode")))).toBe("string");
		expect(checker.typeToString(checker.getBaseTypeOfLiteralType(typeOf("one")))).toBe("number");
		// tsc widens `null` and `undefined`, never literals.
		expect(checker.getWidenedType(typeOf("one"))).toBe(typeOf("one"));
		const named = checker.getDeclaredTypeOfSymbol(checker.getSymbolAtLocation(findIdentifier(sourceFile, "Named")));
		expect(named.getBaseTypes().map((base) => checker.typeToString(base))).toEqual(["Base"]);
		expect(checker.getNumberType().getBaseTypes()).toBeUndefined();
		const constraint = checker.getTypeAtLocation(findIdentifier(sourceFile, "T")).getConstraint();
		expect(constraint && checker.typeToString(constraint)).toBe("string");
		expect(typeOf("mode").aliasSymbol?.name).toBe("Mode");
		expect(typeOf("box").aliasSymbol?.name).toBe("Box");
		// The alias names the type; the type's own symbol is its type literal's.
		expect(typeOf("box").symbol?.name).toBe("__type");
		expect(typeOf("one").aliasSymbol).toBeUndefined();
	});
});

describe("configuration and resolution", () => {
	test("config files: read, parse, find", () => {
		const dir = project({
			"tsconfig.json": '{\n  // comment\n  "compilerOptions": { "strict": true, "target": "ES2022", "jsx": "react-jsx" },\n  "include": ["src"],\n}\n',
			"src/a.ts": "export {};\n",
			"src/nested/b.ts": "export {};\n",
		});
		const configPath = ts.findConfigFile(path.join(dir, "src/nested"), ts.sys.fileExists);
		expect(configPath).toBe(path.join(dir, "tsconfig.json"));
		const { config, error } = ts.readConfigFile(configPath, ts.sys.readFile);
		expect(error).toBeUndefined();
		const parsed = ts.parseJsonConfigFileContent(config, ts.sys, dir);
		expect(parsed.errors).toEqual([]);
		expect(parsed.options.target).toBe(ts.ScriptTarget.ES2022);
		expect(parsed.options.jsx).toBe(ts.JsxEmit.ReactJSX);
		expect(parsed.fileNames.map((file) => path.relative(dir, file)).sort()).toEqual(["src/a.ts", "src/nested/b.ts"]);
		expect(ts.readConfigFile(path.join(dir, "missing.json"), ts.sys.readFile).error.code).toBe(5083);
		const program = ts.createProgram({ rootNames: parsed.fileNames, options: parsed.options });
		expect(program.getSourceFiles().filter((file) => !file.isDeclarationFile)).toHaveLength(2);
	});

	test("resolveModuleName uses surge's resolver", () => {
		const dir = project({
			"src/index.ts": "export {};\n",
			"src/util.ts": "export {};\n",
			"node_modules/pkg/package.json": '{ "name": "pkg", "types": "index.d.ts" }',
			"node_modules/pkg/index.d.ts": "export declare const x: number;\n",
		});
		const containing = path.join(dir, "src/index.ts");
		const relative = ts.resolveModuleName("./util", containing, options, ts.sys);
		expect(relative.resolvedModule.resolvedFileName).toBe(path.join(dir, "src/util.ts"));
		expect(relative.resolvedModule.extension).toBe(".ts");
		const pkg = ts.resolveModuleName("pkg", containing, options, ts.sys);
		expect(pkg.resolvedModule.resolvedFileName).toBe(path.join(dir, "node_modules/pkg/index.d.ts"));
		expect(pkg.resolvedModule.isExternalLibraryImport).toBe(true);
		expect(ts.resolveModuleName("./missing", containing, options, ts.sys).resolvedModule).toBeUndefined();
	});
});

describe("diagnostics", () => {
	test("formatting matches TypeScript's", () => {
		const dir = project({ "index.ts": "const n: number = 'x';\n" });
		const program = ts.createProgram([path.join(dir, "index.ts")], options);
		const diagnostics = ts.getPreEmitDiagnostics(program);
		const host = { getCurrentDirectory: () => dir, getCanonicalFileName: (name) => name, getNewLine: () => "\n" };
		expect(ts.formatDiagnostics(diagnostics, host)).toBe(
			"index.ts(1,7): error TS2322: Type 'string' is not assignable to type 'number'.\n",
		);
		const pretty = ts.formatDiagnosticsWithColorAndContext(diagnostics, host);
		expect(pretty).toMatch(/index\.ts.*:.*1.*:.*7/);
		expect(pretty).toContain("const n: number = 'x';");
	});
});
