"use strict";

// tsc's declaration emit for a program (emitter.ts `emitDeclarationFileOrBundle`
// and `printSourceFileOrBundle`): each emitted file's declarations transformed,
// passed through the caller's `afterDeclarations` transformers, printed, and
// written with their source map.
const { SyntaxKind: K, SymbolFlags: SF } = require("../enums.cjs");
const paths = require("../paths.cjs");
const { sys } = require("../sys.cjs");
const { transformDeclarations } = require("./declarations.cjs");
const { DeclarationEmitResolver } = require("./resolver.cjs");
const { createPrinter } = require("./printer.cjs");
const { createTextWriter } = require("./writer.cjs");
const { createSourceMapGenerator } = require("./sourcemap.cjs");
const { createTransformationContext } = require("./context.cjs");
const emitPaths = require("./paths.cjs");

const resolvers = new WeakMap();

function resolverOf(program) {
	let resolver = resolvers.get(program);
	if (!resolver) resolvers.set(program, (resolver = new DeclarationEmitResolver(program)));
	return resolver;
}

function emitHostOf(program) {
	const options = program.getCompilerOptions();
	const useCaseSensitiveFileNames = sys.useCaseSensitiveFileNames;
	const getCanonicalFileName = paths.createGetCanonicalFileName(useCaseSensitiveFileNames);
	let commonSourceDirectory;
	return {
		options,
		getCurrentDirectory: () => program.getCurrentDirectory(),
		getCanonicalFileName,
		getCommonSourceDirectory: () => (commonSourceDirectory ??= emitPaths.getCommonSourceDirectory(program, getCanonicalFileName)),
	};
}

/** tsc's `sourceFileMayBeEmitted` for declarations. */
function mayEmitDeclarations(program, sourceFile) {
	return !sourceFile.isDeclarationFile && !program.isSourceFileFromExternalLibrary(sourceFile) && !sourceFile.fileName.endsWith(".json");
}

/** tsc's `getNewLineCharacter`. */
function newLineOf(options) {
	switch (options.newLine) {
		case 0:
			return "\r\n";
		case 1:
			return "\n";
	}
	return sys.newLine;
}

/**
 * The aliases tsc's checker marks visible for declaration emit while it
 * checks a file (`collectLinkedAliases` from `checkExportSpecifier` and
 * `checkExportAssignment`): what a local `export { x }` or `export = x`
 * names stays in the declaration file.
 */
function collectLinkedAliases(program, resolver, sourceFile) {
	const checker = program.getTypeChecker();
	const meaning = SF.Value | SF.Type | SF.Namespace | SF.Alias;
	const mark = (name) => {
		const text = name.text ?? name.escapedText;
		const symbol = checker.resolveName(text, name, meaning, false);
		if (!symbol) return;
		const visited = new Set([symbol]);
		const markDeclarations = (declarations) => {
			for (const declaration of declarations ?? []) {
				resolver._visible.set(declaration, true);
				if (declaration.kind === K.ImportEqualsDeclaration && declaration.moduleReference.kind !== K.ExternalModuleReference) {
					let first = declaration.moduleReference;
					while (first.kind === K.QualifiedName) first = first.left;
					const imported = checker.resolveName(first.text, declaration, SF.Value | SF.Type | SF.Namespace, false);
					if (imported && !visited.has(imported)) {
						visited.add(imported);
						markDeclarations(imported.declarations);
					}
				}
			}
		};
		markDeclarations(symbol.declarations);
	};
	const visit = (node) => {
		if (node.kind === K.ExportAssignment) {
			if (node.expression.kind === K.Identifier) mark(node.expression);
			return;
		}
		if (node.kind === K.ExportSpecifier) {
			if (!node.parent.parent.moduleSpecifier) {
				const local = node.propertyName ?? node.name;
				if (local.kind === K.Identifier) mark(local);
			}
			return;
		}
		node.forEachChild(visit);
	};
	visit(sourceFile);
}

/** tsc's declaration transform of one file, with what its checker has marked visible. */
function transformDeclarationFile(program, resolver, sourceFile) {
	resolver._linked ??= new Set();
	if (!resolver._linked.has(sourceFile)) {
		resolver._linked.add(sourceFile);
		collectLinkedAliases(program, resolver, sourceFile);
	}
	const options = program.getCompilerOptions();
	return transformDeclarations(sourceFile, { resolver, isExternalModule: sourceFile.externalModuleIndicator !== undefined, stripInternal: options.stripInternal });
}

/**
 * tsc's `getDeclarationDiagnostics` for one file: what its declaration
 * transform reports (an exported name reaching something declaration files
 * cannot name). Kept per file, as tsc keeps them. A file whose declarations
 * surge-ts cannot build yet reports nothing here; emitting it says so.
 */
function declarationDiagnosticsOf(program, sourceFile) {
	if (!mayEmitDeclarations(program, sourceFile)) return [];
	const resolver = resolverOf(program);
	resolver._diagnostics ??= new Map();
	let diagnostics = resolver._diagnostics.get(sourceFile);
	if (diagnostics === undefined) {
		try {
			diagnostics = transformDeclarationFile(program, resolver, sourceFile).diagnostics;
		} catch (error) {
			if (!/^surge-ts: .*not supported/.test(error?.message ?? "")) throw error;
			diagnostics = [];
		}
		resolver._diagnostics.set(sourceFile, diagnostics);
	}
	return diagnostics;
}

/**
 * `Program.emit` for declarations: the declaration file (and map) of
 * `targetSourceFile`, or of every file the program emits.
 */
function emitDeclarations(program, targetSourceFile, writeFile, customTransformers) {
	const options = program.getCompilerOptions();
	const host = emitHostOf(program);
	const resolver = resolverOf(program);
	const newLine = newLineOf(options);
	const diagnostics = [];
	let emitSkipped = false;
	const files = targetSourceFile ? [targetSourceFile] : program.getSourceFiles().filter((file) => mayEmitDeclarations(program, file));
	for (const sourceFile of files) {
		if (!mayEmitDeclarations(program, sourceFile)) continue;
		const { declarationFilePath, declarationMapPath } = emitPaths.getDeclarationOutputPaths(sourceFile.fileName, options, host);
		// The declaration errors surge's check makes are ones tsc's transform reports, which block the file's emit.
		const checked = program._diagnostics("declaration", sourceFile);
		const result = checked.length ? { diagnostics: checked } : transformDeclarationFile(program, resolver, sourceFile);
		if (result.diagnostics.length) {
			diagnostics.push(...result.diagnostics);
			emitSkipped = true;
			continue;
		}
		let transformed = result.transformed;
		const afterDeclarations = customTransformers?.afterDeclarations ?? [];
		if (afterDeclarations.length) {
			const context = createTransformationContext(program, diagnostics);
			for (const factory of afterDeclarations) {
				const transformer = typeof factory === "function" ? factory(context) : factory.transformSourceFile ? factory : undefined;
				transformed = typeof transformer === "function" ? transformer(transformed) : transformer.transformSourceFile(transformed);
			}
		}
		if (options.noEmit) {
			emitSkipped = true;
			continue;
		}
		printDeclarationFile(transformed, declarationFilePath, declarationMapPath, { options, host, newLine, writeFile, sourceFile });
	}
	return { emitSkipped, diagnostics, emittedFiles: undefined, sourceMaps: undefined };
}

/** tsc's `printSourceFileOrBundle` for one declaration file. */
function printDeclarationFile(transformed, declarationFilePath, declarationMapPath, { options, host, newLine, writeFile, sourceFile }) {
	const printer = createPrinter({
		removeComments: options.removeComments,
		newLine: options.newLine,
		noEmitHelpers: true,
		module: options.module,
		target: options.target,
		sourceMap: options.declarationMap,
		inlineSourceMap: options.inlineSourceMap,
		onlyPrintJsDocStyle: true,
		omitBraceSourceMapPositions: true,
	});
	const writer = createTextWriter(newLine);
	let generator;
	if ((options.declarationMap || options.inlineSourceMap) && !transformed.fileName.endsWith(".json")) {
		generator = createSourceMapGenerator(
			host,
			paths.getBaseFileName(paths.normalizeSlashes(declarationFilePath)),
			emitPaths.getSourceRoot(options),
			emitPaths.getSourceMapDirectory(options, declarationFilePath, sourceFile, host),
		);
	}
	printer.writeFile(transformed, writer, generator);
	if (generator) {
		if (options.inlineSourceMap) throw new Error("surge-ts: declaration emit with inlineSourceMap is not supported yet");
		const sourceMappingURL = emitPaths.getSourceMappingURL(options, declarationFilePath, declarationMapPath, sourceFile, host);
		if (sourceMappingURL) {
			if (!writer.isAtStartOfLine()) writer.rawWrite(newLine);
			writer.writeComment(`//# sourceMappingURL=${sourceMappingURL}`);
		}
		if (declarationMapPath) write(writeFile, declarationMapPath, generator.toString(), [sourceFile]);
	} else {
		writer.writeLine();
	}
	write(writeFile, declarationFilePath, writer.getText(), [sourceFile], options.emitBOM);
}

function write(writeFile, fileName, text, sourceFiles, writeByteOrderMark = false) {
	if (writeFile) {
		writeFile(fileName, text, writeByteOrderMark, undefined, sourceFiles, undefined);
		return;
	}
	const directory = paths.getDirectoryPath(fileName);
	if (!sys.directoryExists(directory)) require("node:fs").mkdirSync(directory, { recursive: true });
	sys.writeFile(fileName, text, writeByteOrderMark);
}

module.exports = { emitDeclarations, declarationDiagnosticsOf, mayEmitDeclarations, resolverOf };
