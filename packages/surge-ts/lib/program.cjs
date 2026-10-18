"use strict";

// `createProgram`, `Program` and `createSourceFile`, TypeScript-shaped, over
// the native program.
const native = require("./native.cjs");
const { ScriptKind, ScriptTarget } = require("./enums.cjs");
const { SourceFileObject } = require("./nodes.cjs");
const { TypeChecker, bindPublicMethods } = require("./checker.cjs");
const { sys } = require("./sys.cjs");
const internals = require("./internals.cjs");

/** A native program file, as the node layer reads it. */
function fileBacking(nativeProgram, fileIndex) {
	return {
		nodeTable: () => nativeProgram.nodeTable(fileIndex),
		nodeProperties: (index) => nativeProgram.nodeProperties(fileIndex, index),
		nodeScalars: (index) => nativeProgram.nodeScalars(fileIndex, index),
		text: () => nativeProgram.sourceFileText(fileIndex),
		lineStarts: () => nativeProgram.lineStarts(fileIndex),
	};
}

function toDiagnostic(program, record) {
	const diagnostic = {
		file: record.file === null || record.file === undefined ? undefined : program._sourceFile(record.file),
		start: record.start ?? undefined,
		length: record.length ?? undefined,
		code: record.code,
		category: record.category,
		messageText: record.messageText,
	};
	if (record.surgeCode) diagnostic.surgeCode = record.surgeCode;
	if (record.fileName) diagnostic.fileName = record.fileName;
	return diagnostic;
}

function fromDiagnostic(diagnostic) {
	return {
		file: undefined,
		start: diagnostic.start,
		length: diagnostic.length,
		code: diagnostic.code ?? 0,
		category: diagnostic.category ?? 1,
		messageText: typeof diagnostic.messageText === "string" ? diagnostic.messageText : require("./diagnostics.cjs").flattenDiagnosticMessageText(diagnostic.messageText, "\n"),
		surgeCode: diagnostic.surgeCode,
		fileName: diagnostic.file?.fileName ?? diagnostic.fileName,
	};
}

class Program {
	constructor(nativeProgram) {
		this._native = nativeProgram;
		this._sourceFiles = new Array(nativeProgram.sourceFileCount());
		this._checker = undefined;
		this._options = undefined;
		bindPublicMethods(this, Program.prototype);
	}
	/** @internal */
	_sourceFile(index) {
		let sourceFile = this._sourceFiles[index];
		if (sourceFile === undefined) {
			const info = this._native.sourceFileInfo(index);
			sourceFile = new SourceFileObject(fileBacking(this._native, index), info, this, index);
			sourceFile._info = info;
			this._sourceFiles[index] = sourceFile;
		}
		return sourceFile;
	}
	/** @internal */
	_nodeOf(reference) {
		return this._sourceFile(reference.file)._node(reference.node);
	}
	_fileIndex(sourceFile) {
		if (sourceFile === undefined) return undefined;
		if (!(sourceFile instanceof SourceFileObject) || sourceFile._program !== this) {
			throw new TypeError("surge-ts: the source file belongs to another program");
		}
		return sourceFile._fileIndex;
	}
	_diagnostics(kind, sourceFile) {
		return this._native.diagnostics(kind, this._fileIndex(sourceFile)).map((record) => toDiagnostic(this, record));
	}
	getRootFileNames() {
		return this._native.rootFileNames();
	}
	getSourceFiles() {
		const files = [];
		for (let index = 0; index < this._sourceFiles.length; index++) files.push(this._sourceFile(index));
		return files;
	}
	getSourceFile(fileName) {
		const index = this._native.sourceFileIndex(fileName);
		return index === null || index === undefined ? undefined : this._sourceFile(index);
	}
	getSourceFileByPath(path) {
		return this.getSourceFile(path);
	}
	getCompilerOptions() {
		if (this._options === undefined) this._options = this._native.compilerOptions();
		return this._options;
	}
	getCurrentDirectory() {
		return this._native.currentDirectory();
	}
	getTypeChecker() {
		if (this._checker === undefined) this._checker = new TypeChecker(this);
		return this._checker;
	}
	getSyntacticDiagnostics(sourceFile) {
		return this._diagnostics("syntactic", sourceFile);
	}
	getSemanticDiagnostics(sourceFile) {
		return this._diagnostics("semantic", sourceFile);
	}
	getDeclarationDiagnostics(sourceFile) {
		return this._diagnostics("declaration", sourceFile);
	}
	getGlobalDiagnostics() {
		return this._diagnostics("global");
	}
	getOptionsDiagnostics() {
		return this._diagnostics("options");
	}
	getConfigFileParsingDiagnostics() {
		return this._diagnostics("config");
	}
	isSourceFileFromExternalLibrary(sourceFile) {
		return this._sourceFile(this._fileIndex(sourceFile))._info.isExternalLibrary;
	}
	isSourceFileDefaultLibrary(sourceFile) {
		return this._sourceFile(this._fileIndex(sourceFile))._info.isDefaultLibrary;
	}
	/**
	 * The module `moduleName` imported from `sourceFile` resolved to in this
	 * program, as `{ resolvedModule }`; `undefined` when it did not resolve.
	 */
	getResolvedModule(sourceFile, moduleName) {
		const index = this._native.resolvedModule(this._fileIndex(sourceFile), moduleName);
		if (index === null || index === undefined) return undefined;
		const fileName = this._sourceFile(index).fileName;
		return {
			resolvedModule: {
				resolvedFileName: fileName,
				extension: extensionOf(fileName),
				isExternalLibraryImport: fileName.includes("/node_modules/"),
			},
		};
	}
	emit() {
		throw new Error("surge-ts: emit is not supported; surge-ts is a type checker (noEmit)");
	}
	/** TypeScript-internal: the directory output paths are computed from. */
	getCommonSourceDirectory() {
		return internals.getCommonSourceDirectory(this);
	}
	/** TypeScript-internal: the program's files reached through symbolic links. */
	getSymlinkCache() {
		this._symlinkCache ??= internals.getSymlinkCache(this);
		return this._symlinkCache;
	}
}

function extensionOf(fileName) {
	const lower = fileName.toLowerCase();
	for (const extension of [".d.ts", ".d.mts", ".d.cts", ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"]) {
		if (lower.endsWith(extension)) return extension;
	}
	return "";
}

const SYSTEM_HOST = Symbol("surge-ts.systemHost");

function isScriptFile(fileName) {
	return /\.(?:[cm]?tsx?|[cm]?jsx?)$/i.test(fileName);
}

/** The callbacks the native loader reads a user's `CompilerHost` through. */
function hostCallbacks(host, options) {
	const defaults = host[SYSTEM_HOST];
	const overridesGetSourceFile = typeof host.getSourceFile === "function" && host.getSourceFile !== defaults?.getSourceFile;
	return {
		readFile(fileName) {
			if (overridesGetSourceFile && isScriptFile(fileName)) {
				const sourceFile = host.getSourceFile(fileName, options.target ?? ScriptTarget.Latest);
				if (sourceFile) return sourceFile.text;
			}
			return host.readFile(fileName) ?? null;
		},
		fileExists: (fileName) => Boolean(host.fileExists(fileName)),
		directoryExists: host.directoryExists ? (directoryName) => Boolean(host.directoryExists(directoryName)) : undefined,
		getDirectories: host.getDirectories ? (directoryName) => host.getDirectories(directoryName) ?? [] : undefined,
		realpath: host.realpath ? (name) => host.realpath(name) : undefined,
		currentDirectory: host.getCurrentDirectory(),
		useCaseSensitiveFileNames: Boolean(host.useCaseSensitiveFileNames?.()),
		newLine: host.getNewLine?.() ?? "\n",
	};
}

/** Whether a host reads the file system exactly as the native loader does. */
function isSystemHost(host) {
	const defaults = host[SYSTEM_HOST];
	if (!defaults) return false;
	return ["readFile", "fileExists", "directoryExists", "getDirectories", "realpath", "getSourceFile", "getCurrentDirectory"].every(
		(method) => host[method] === defaults[method],
	);
}

/**
 * `createProgram(rootNames, options, host?, oldProgram?, configFileParsingDiagnostics?)`
 * or `createProgram({ rootNames, options, host?, configFileParsingDiagnostics? })`.
 * `oldProgram` is accepted and not reused: every program is built afresh.
 */
function createProgram(rootNamesOrOptions, options, host, oldProgram, configFileParsingDiagnostics) {
	const create = Array.isArray(rootNamesOrOptions)
		? { rootNames: rootNamesOrOptions, options, host, oldProgram, configFileParsingDiagnostics }
		: rootNamesOrOptions;
	const compilerOptions = create.options ?? {};
	const nativeProgram = native.NativeProgram.create({
		rootNames: [...create.rootNames],
		options: JSON.parse(JSON.stringify(compilerOptions)),
		host: create.host && !isSystemHost(create.host) ? hostCallbacks(create.host, compilerOptions) : undefined,
		configFileParsingDiagnostics: (create.configFileParsingDiagnostics ?? []).map(fromDiagnostic),
	});
	return new Program(nativeProgram);
}

/**
 * `createSourceFile`: `text` parsed on its own, with no program. The file is
 * parsed as its name says (TypeScript's `getScriptKindFromFileName`).
 */
function createSourceFile(fileName, sourceText, languageVersionOrOptions, setParentNodes, scriptKind) {
	if (scriptKind && scriptKind !== (scriptKindFromFileName(fileName) || ScriptKind.TS)) {
		throw new Error("surge-ts: createSourceFile with a scriptKind other than its file name's is not supported yet");
	}
	const program = new Program(native.NativeProgram.parseSourceFile(fileName, sourceText));
	return program._sourceFile(0);
}

function scriptKindFromFileName(fileName) {
	switch (fileName.slice(fileName.lastIndexOf(".")).toLowerCase()) {
		case ".js":
		case ".cjs":
		case ".mjs":
			return ScriptKind.JS;
		case ".jsx":
			return ScriptKind.JSX;
		case ".ts":
		case ".cts":
		case ".mts":
			return ScriptKind.TS;
		case ".tsx":
			return ScriptKind.TSX;
		case ".json":
			return ScriptKind.JSON;
		default:
			return ScriptKind.Unknown;
	}
}

/** `createCompilerHost`: a host over `ts.sys`. */
function createCompilerHost(options, setParentNodes) {
	const host = {
		getSourceFile(fileName, languageVersionOrOptions, onError) {
			const text = host.readFile(fileName);
			if (text === undefined) {
				onError?.(`Cannot read file '${fileName}'.`);
				return undefined;
			}
			return createSourceFile(fileName, text, languageVersionOrOptions, setParentNodes);
		},
		getDefaultLibFileName: (compilerOptions) => getDefaultLibFileName(compilerOptions),
		getDefaultLibLocation: () => "",
		writeFile: (fileName, data, writeByteOrderMark) => sys.writeFile(fileName, data, writeByteOrderMark),
		getCurrentDirectory: () => sys.getCurrentDirectory(),
		getDirectories: (path) => sys.getDirectories(path),
		getCanonicalFileName: (fileName) => (sys.useCaseSensitiveFileNames ? fileName : fileName.toLowerCase()),
		useCaseSensitiveFileNames: () => sys.useCaseSensitiveFileNames,
		getNewLine: () => sys.newLine,
		fileExists: (fileName) => sys.fileExists(fileName),
		readFile: (fileName) => sys.readFile(fileName),
		directoryExists: (directoryName) => sys.directoryExists(directoryName),
		realpath: (name) => sys.realpath(name),
	};
	Object.defineProperty(host, SYSTEM_HOST, { value: { ...host } });
	return host;
}

/** tsc's `getDefaultLibFileName`. */
function getDefaultLibFileName(options) {
	switch (options?.target) {
		case ScriptTarget.ESNext:
			return "lib.esnext.full.d.ts";
		case ScriptTarget.ES2025:
			return "lib.es2025.full.d.ts";
		case ScriptTarget.ES2024:
			return "lib.es2024.full.d.ts";
		case ScriptTarget.ES2023:
			return "lib.es2023.full.d.ts";
		case ScriptTarget.ES2022:
			return "lib.es2022.full.d.ts";
		case ScriptTarget.ES2021:
			return "lib.es2021.full.d.ts";
		case ScriptTarget.ES2020:
			return "lib.es2020.full.d.ts";
		case ScriptTarget.ES2019:
			return "lib.es2019.full.d.ts";
		case ScriptTarget.ES2018:
			return "lib.es2018.full.d.ts";
		case ScriptTarget.ES2017:
			return "lib.es2017.full.d.ts";
		case ScriptTarget.ES2016:
			return "lib.es2016.full.d.ts";
		case ScriptTarget.ES2015:
			return "lib.es6.d.ts";
		default:
			return "lib.d.ts";
	}
}

/** `getPreEmitDiagnostics`. */
function getPreEmitDiagnostics(program, sourceFile) {
	if (!(program instanceof Program)) throw new TypeError("surge-ts: expected a surge-ts program");
	return program._diagnostics("preEmit", sourceFile);
}

module.exports = { Program, createProgram, createSourceFile, createCompilerHost, getDefaultLibFileName, getPreEmitDiagnostics };
