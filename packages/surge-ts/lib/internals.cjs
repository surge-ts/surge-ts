"use strict";

// Members of TypeScript's nodes, source files and programs that TypeScript
// keeps internal (`node.symbol`, `sourceFile.commentDirectives`,
// `program.getCommonSourceDirectory()`, …), installed on surge-ts's objects.

const { SyntaxKind, SymbolFlags, CommentDirectiveType } = require("./enums.cjs");
const { NodeObject, SourceFileObject, forEachChild, escapeLeadingUnderscores } = require("./nodes.cjs");
const { sys } = require("./sys.cjs");
const paths = require("./paths.cjs");
const { getLeadingCommentRanges, isLineBreak } = require("./utilities.cjs");

const K = SyntaxKind;


// --- comment directives ----------------------------------------------------

const singleLineDirective = /^\/\/\/?\s*@(ts-expect-error|ts-ignore)/;
const multiLineDirective = /^(?:\/|\*)*\s*@(ts-expect-error|ts-ignore)/;

function directiveType(text, pattern) {
	const match = pattern.exec(text.trimStart());
	if (!match) return undefined;
	return match[1] === "ts-expect-error" ? CommentDirectiveType.ExpectError : CommentDirectiveType.Ignore;
}

/**
 * The text spans a file's comments can be in. Every literal (string,
 * template part, regular expression, JSX text) is a leaf node, so outside
 * leaves the text is trivia, punctuation and keywords only.
 */
function leafSpans(sourceFile) {
	const spans = [];
	(function visit(node) {
		let leaf = true;
		forEachChild(node, (child) => {
			leaf = false;
			visit(child);
		});
		if (leaf && node !== sourceFile) spans.push([node.getStart(sourceFile), node.end]);
	})(sourceFile);
	return spans.sort((a, b) => a[0] - b[0]);
}

/** tsc scanner's `commentDirectives`: every `@ts-expect-error` / `@ts-ignore` comment. */
function collectCommentDirectives(sourceFile) {
	const text = sourceFile.text;
	const spans = leafSpans(sourceFile);
	let directives;
	let span = 0;
	let position = 0;
	if (text.startsWith("#!")) {
		while (position < text.length && !isLineBreak(text.charCodeAt(position))) position++;
	}
	while (position < text.length) {
		while (span < spans.length && spans[span][1] <= position) span++;
		if (span < spans.length && spans[span][0] <= position) {
			position = spans[span][1];
			continue;
		}
		if (text.charCodeAt(position) !== 47) {
			position++;
			continue;
		}
		const next = text.charCodeAt(position + 1);
		const start = position;
		if (next === 47) {
			position += 2;
			while (position < text.length && !isLineBreak(text.charCodeAt(position))) position++;
			const type = directiveType(text.slice(start, position), singleLineDirective);
			if (type !== undefined) (directives ??= []).push({ range: { pos: start, end: position }, type });
		} else if (next === 42) {
			position += 2;
			let lastLineStart = start;
			while (position < text.length) {
				const ch = text.charCodeAt(position);
				if (ch === 42 && text.charCodeAt(position + 1) === 47) {
					position += 2;
					break;
				}
				position++;
				if (isLineBreak(ch)) lastLineStart = position;
			}
			const type = directiveType(text.slice(lastLineStart, position), multiLineDirective);
			if (type !== undefined) (directives ??= []).push({ range: { pos: lastLineStart, end: position }, type });
		} else {
			position++;
		}
	}
	return directives;
}

// --- pragmas (the parser's processCommentPragmas) ---------------------------

const PragmaKind = { TripleSlashXML: 1, SingleLine: 2, MultiLine: 4 };
const commentPragmas = {
	reference: {
		args: [
			{ name: "types", optional: true, captureSpan: true },
			{ name: "lib", optional: true, captureSpan: true },
			{ name: "path", optional: true, captureSpan: true },
			{ name: "no-default-lib", optional: true },
			{ name: "resolution-mode", optional: true },
			{ name: "preserve", optional: true },
		],
		kind: PragmaKind.TripleSlashXML,
	},
	"amd-dependency": { args: [{ name: "path" }, { name: "name", optional: true }], kind: PragmaKind.TripleSlashXML },
	"amd-module": { args: [{ name: "name" }], kind: PragmaKind.TripleSlashXML },
	"ts-check": { kind: PragmaKind.SingleLine },
	"ts-nocheck": { kind: PragmaKind.SingleLine },
	jsx: { args: [{ name: "factory" }], kind: PragmaKind.MultiLine },
	jsxfrag: { args: [{ name: "factory" }], kind: PragmaKind.MultiLine },
	jsximportsource: { args: [{ name: "factory" }], kind: PragmaKind.MultiLine },
	jsxruntime: { args: [{ name: "factory" }], kind: PragmaKind.MultiLine },
};

const tripleSlashXMLCommentStart = /^\/\/\/\s*<(\S+)\s.*?\/>/m;
const singleLinePragma = /^\/\/\/?\s*@([^\s:]+)((?:[^\S\r\n]|:).*)?$/m;
const namedArgPatterns = new Map();

function namedArgPattern(name) {
	let pattern = namedArgPatterns.get(name);
	if (!pattern) {
		pattern = new RegExp(`(\\s${name}\\s*=\\s*)(?:(?:'([^']*)')|(?:"([^"]*)"))`, "im");
		namedArgPatterns.set(name, pattern);
	}
	return pattern;
}

function namedPragmaArguments(pragma, text) {
	if (!text || !pragma.args) return {};
	const values = text.trim().split(/\s+/);
	const result = {};
	for (let index = 0; index < pragma.args.length; index++) {
		const argument = pragma.args[index];
		if (!values[index] && !argument.optional) return "fail";
		result[argument.name] = values[index];
	}
	return result;
}

function addPragmaForMatch(pragmas, range, kind, match) {
	const name = match[1].toLowerCase();
	const pragma = commentPragmas[name];
	if (!pragma || !(pragma.kind & kind)) return;
	const args = namedPragmaArguments(pragma, match[2]);
	if (args !== "fail") pragmas.push({ name, args: { arguments: args, range } });
}

function extractPragmas(pragmas, range, text) {
	const tripleSlash = range.kind === K.SingleLineCommentTrivia && tripleSlashXMLCommentStart.exec(text);
	if (tripleSlash) {
		const name = tripleSlash[1].toLowerCase();
		const pragma = commentPragmas[name];
		if (!pragma || !(pragma.kind & PragmaKind.TripleSlashXML)) return;
		const args = {};
		for (const argument of pragma.args ?? []) {
			const match = namedArgPattern(argument.name).exec(text);
			if (!match && !argument.optional) return;
			if (!match) continue;
			const value = match[2] || match[3];
			if (argument.captureSpan) {
				const pos = range.pos + match.index + match[1].length + 1;
				args[argument.name] = { value, pos, end: pos + value.length };
			} else {
				args[argument.name] = value;
			}
		}
		pragmas.push({ name, args: { arguments: args, range } });
		return;
	}
	const singleLine = range.kind === K.SingleLineCommentTrivia && singleLinePragma.exec(text);
	if (singleLine) return addPragmaForMatch(pragmas, range, PragmaKind.SingleLine, singleLine);
	if (range.kind === K.MultiLineCommentTrivia) {
		const multiLine = /@(\S+)(\s+(?:\S.*)?)?$/gm;
		let match;
		while ((match = multiLine.exec(text))) addPragmaForMatch(pragmas, range, PragmaKind.MultiLine, match);
	}
}

function collectPragmas(text) {
	const pragmas = [];
	for (const range of getLeadingCommentRanges(text, 0) ?? []) extractPragmas(pragmas, range, text.substring(range.pos, range.end));
	const map = new Map();
	for (const { name, args } of pragmas) {
		const current = map.get(name);
		if (current === undefined) map.set(name, args);
		else if (Array.isArray(current)) current.push(args);
		else map.set(name, [current, args]);
	}
	return map;
}

// --- entity names -----------------------------------------------------------

const identifierPattern = /^[\p{ID_Start}$_][\p{ID_Continue}$‌‍]*$/u;

/**
 * `parseIsolatedEntityName`: `text` as an entity name (`React.createElement`),
 * or `undefined` when it is not one. Its nodes have no parent, as
 * TypeScript's do.
 */
function parseIsolatedEntityName(text) {
	const parts = text.split(".");
	let entity;
	let position = 0;
	for (const part of parts) {
		const name = part.trim();
		if (!identifierPattern.test(name)) return undefined;
		const pos = position;
		position += part.length + 1;
		const identifier = { kind: K.Identifier, pos, end: pos + part.length, flags: 0, text: name, escapedText: escapeLeadingUnderscores(name) };
		entity = entity ? { kind: K.QualifiedName, pos: 0, end: identifier.end, flags: 0, left: entity, right: identifier } : identifier;
	}
	return entity;
}

// --- installation -----------------------------------------------------------

const commentDirectivesCache = new WeakMap();
const pragmasCache = new WeakMap();

/**
 * tsc's `processPragmasIntoFields` for `/// <reference>`: the files, type
 * packages and libs a file names, each a `FileReference` spanning the
 * attribute's value, and whether it has `no-default-lib`.
 */
function referenceFields(sourceFile) {
	const fields = { referencedFiles: [], typeReferenceDirectives: [], libReferenceDirectives: [], hasNoDefaultLib: false };
	let references = sourceFile.pragmas.get("reference");
	if (references !== undefined && !Array.isArray(references)) references = [references];
	for (const { arguments: args } of references ?? []) {
		const preserve = args.preserve === "true" ? { preserve: true } : {};
		if (args["no-default-lib"] === "true") {
			fields.hasNoDefaultLib = true;
		} else if (args.types) {
			const mode = args["resolution-mode"];
			const resolutionMode = mode === "require" ? { resolutionMode: 1 } : mode === "import" ? { resolutionMode: 99 } : {};
			fields.typeReferenceDirectives.push({ pos: args.types.pos, end: args.types.end, fileName: args.types.value, ...resolutionMode, ...preserve });
		} else if (args.lib) {
			fields.libReferenceDirectives.push({ pos: args.lib.pos, end: args.lib.end, fileName: args.lib.value, ...preserve });
		} else if (args.path) {
			fields.referencedFiles.push({ pos: args.path.pos, end: args.path.end, fileName: args.path.value, ...preserve });
		}
	}
	return fields;
}

// Read once, then the file's own: tools rewrite them in place.
const referenceFieldDescriptors = Object.fromEntries(
	["referencedFiles", "typeReferenceDirectives", "libReferenceDirectives", "hasNoDefaultLib"].map((name) => [
		name,
		{
			configurable: true,
			get() {
				const fields = referenceFields(this);
				for (const [field, value] of Object.entries(fields)) {
					if (Object.hasOwn(this, field)) continue;
					Object.defineProperty(this, field, { configurable: true, enumerable: true, writable: true, value });
				}
				return fields[name];
			},
			set(value) {
				Object.defineProperty(this, name, { configurable: true, enumerable: true, writable: true, value });
			},
		},
	]),
);

Object.defineProperties(SourceFileObject.prototype, referenceFieldDescriptors);

Object.defineProperties(SourceFileObject.prototype, {
	commentDirectives: {
		configurable: true,
		get() {
			if (!commentDirectivesCache.has(this)) commentDirectivesCache.set(this, collectCommentDirectives(this));
			return commentDirectivesCache.get(this);
		},
	},
	pragmas: {
		configurable: true,
		get() {
			let pragmas = pragmasCache.get(this);
			if (pragmas === undefined) {
				pragmas = collectPragmas(this.text);
				pragmasCache.set(this, pragmas);
			}
			return pragmas;
		},
	},
	path: {
		configurable: true,
		get() {
			const directory = this._program?.getCurrentDirectory?.() ?? sys.getCurrentDirectory();
			return paths.toPath(this.fileName, directory, paths.createGetCanonicalFileName(sys.useCaseSensitiveFileNames));
		},
	},
	resolvedPath: {
		configurable: true,
		get() {
			return this.path;
		},
	},
	originalFileName: {
		configurable: true,
		get() {
			return this.fileName;
		},
	},
});

// `node.symbol`: the binder's symbol of a declaration. A source file parsed
// on its own (`createSourceFile`) has no checker and no symbols.
Object.defineProperty(NodeObject.prototype, "symbol", {
	configurable: true,
	get() {
		const program = this._sourceFile._program;
		if (!program?._native?.declarationSymbol) return undefined;
		const checker = program.getTypeChecker();
		return checker._symbolOrUndefined(program._native.declarationSymbol(this._sourceFile._fileIndex, this._index));
	},
});

/** tsc's `computeCommonSourceDirectoryOfFilenames`. */
function commonDirectoryOfFiles(fileNames, currentDirectory, getCanonicalFileName) {
	let common;
	for (const fileName of fileNames) {
		const components = paths.reducePathComponents(paths.getPathComponents(fileName, currentDirectory));
		components.pop();
		if (!common) {
			common = components;
			continue;
		}
		const length = Math.min(common.length, components.length);
		for (let index = 0; index < length; index++) {
			if (getCanonicalFileName(common[index]) !== getCanonicalFileName(components[index])) {
				if (index === 0) return "";
				common.length = index;
				break;
			}
		}
		if (components.length < common.length) common.length = components.length;
	}
	return common ? paths.getPathFromPathComponents(common) : currentDirectory;
}

/** `Program.getCommonSourceDirectory`. */
function getCommonSourceDirectory(program) {
	const options = program.getCompilerOptions();
	const currentDirectory = program.getCurrentDirectory();
	let directory;
	if (options.rootDir) directory = paths.getNormalizedAbsolutePath(options.rootDir, currentDirectory);
	else if (options.configFilePath) directory = paths.getDirectoryPath(paths.normalizeSlashes(options.configFilePath));
	else {
		const emitted = program
			.getSourceFiles()
			.filter((file) => !file.isDeclarationFile && !program.isSourceFileFromExternalLibrary(file) && !file.fileName.endsWith(".json"))
			.map((file) => file.fileName);
		directory = commonDirectoryOfFiles(emitted, currentDirectory, paths.createGetCanonicalFileName(sys.useCaseSensitiveFileNames));
	}
	return directory && !directory.endsWith(paths.directorySeparator) ? directory + paths.directorySeparator : directory;
}

const isNodeModulesOrScopedPackageDirectory = (component, getCanonicalFileName) =>
	component !== undefined && (getCanonicalFileName(component) === "node_modules" || component.startsWith("@"));

/**
 * tsc's `guessDirectorySymlink`: the real and the linked directory a file's
 * two paths share everything below, short of a package directory.
 */
function guessDirectorySymlink(real, linked, currentDirectory, getCanonicalFileName) {
	const realParts = paths.getPathComponents(paths.getNormalizedAbsolutePath(real, currentDirectory));
	const linkedParts = paths.getPathComponents(paths.getNormalizedAbsolutePath(linked, currentDirectory));
	let isDirectory = false;
	while (
		realParts.length >= 2 &&
		linkedParts.length >= 2 &&
		!isNodeModulesOrScopedPackageDirectory(realParts[realParts.length - 2], getCanonicalFileName) &&
		!isNodeModulesOrScopedPackageDirectory(linkedParts[linkedParts.length - 2], getCanonicalFileName) &&
		getCanonicalFileName(realParts[realParts.length - 1]) === getCanonicalFileName(linkedParts[linkedParts.length - 1])
	) {
		realParts.pop();
		linkedParts.pop();
		isDirectory = true;
	}
	return isDirectory ? [paths.getPathFromPathComponents(realParts), paths.getPathFromPathComponents(linkedParts)] : undefined;
}

/**
 * `Program.getSymlinkCache`: the program's files reached through a symbolic
 * link. Only `getSymlinkedDirectoriesByRealpath` is answered: each real
 * directory (with a trailing `/`, as a `Path`) and the linked directories
 * that lead to it.
 */
function getSymlinkCache(program) {
	let byRealpath;
	return {
		getSymlinkedDirectoriesByRealpath() {
			if (byRealpath === undefined) {
				const getCanonicalFileName = paths.createGetCanonicalFileName(sys.useCaseSensitiveFileNames);
				const currentDirectory = program.getCurrentDirectory();
				byRealpath = new Map();
				for (const file of program.getSourceFiles()) {
					const real = sys.realpath ? paths.normalizeSlashes(sys.realpath(file.fileName)) : file.fileName;
					if (real === file.fileName) continue;
					const guess = guessDirectorySymlink(real, file.fileName, currentDirectory, getCanonicalFileName);
					if (!guess) continue;
					const [target, linked] = guess;
					const key = paths.ensureTrailingDirectorySeparator(paths.toPath(target, currentDirectory, getCanonicalFileName));
					const entries = byRealpath.get(key) ?? [];
					if (!entries.includes(linked)) entries.push(linked);
					byRealpath.set(key, entries);
				}
			}
			return byRealpath.size > 0 ? byRealpath : undefined;
		},
	};
}

module.exports = {
	parseIsolatedEntityName,
	getCommonSourceDirectory,
	getSymlinkCache,
	collectCommentDirectives,
	collectPragmas,
	isAliasSymbol: (symbol) => (symbol.flags & SymbolFlags.Alias) !== 0,
};
