"use strict";

// surge-ts: an embeddable TypeScript type checker written in Rust, with a
// familiar TypeScript Compiler API. `require("surge-ts")` (or
// `import ts from "surge-ts"`) is a namespace shaped like TypeScript's.
// Supported and unsupported APIs are listed in docs/COMPILER_API.md.

const native = require("./lib/native.cjs");
const enums = require("./lib/enums.cjs");
const guards = require("./lib/guards.cjs");
const { sys } = require("./lib/sys.cjs");
const nodes = require("./lib/nodes.cjs");
const program = require("./lib/program.cjs");
const config = require("./lib/config.cjs");
const diagnostics = require("./lib/diagnostics.cjs");

const ts = {
	version: require("./package.json").version,
	versionMajorMinor: require("./package.json").version.split(".").slice(0, 2).join("."),
	...enums,
	sys,
	createProgram: program.createProgram,
	createSourceFile: program.createSourceFile,
	createCompilerHost: program.createCompilerHost,
	getDefaultLibFileName: program.getDefaultLibFileName,
	getPreEmitDiagnostics: program.getPreEmitDiagnostics,
	forEachChild: nodes.forEachChild,
	getLineAndCharacterOfPosition: (sourceFile, position) => sourceFile.getLineAndCharacterOfPosition(position),
	getPositionOfLineAndCharacter: (sourceFile, line, character) => sourceFile.getPositionOfLineAndCharacter(line, character),
	computeLineStarts: nodes.computeLineStarts,
	computeLineAndCharacterOfPosition: nodes.computeLineAndCharacterOfPosition,
	getLineStarts: (sourceFile) => sourceFile.getLineStarts(),
	escapeLeadingUnderscores: nodes.escapeLeadingUnderscores,
	unescapeLeadingUnderscores: nodes.unescapeLeadingUnderscores,
	idText: (identifier) => nodes.unescapeLeadingUnderscores(identifier.escapedText),
	symbolName: (symbol) => symbol.name,
	readConfigFile: config.readConfigFile,
	parseConfigFileTextToJson: config.parseConfigFileTextToJson,
	parseJsonConfigFileContent: config.parseJsonConfigFileContent,
	findConfigFile: config.findConfigFile,
	getParsedCommandLineOfConfigFile: config.getParsedCommandLineOfConfigFile,
	resolveModuleName(moduleName, containingFile, compilerOptions, host) {
		if (host && host.fileExists !== sys.fileExists && host.readFile !== sys.readFile) {
			throw new Error("surge-ts: resolveModuleName reads the file system; a custom ModuleResolutionHost is not supported yet");
		}
		const resolved = native.resolveModuleName(moduleName, containingFile, JSON.parse(JSON.stringify(compilerOptions ?? {})));
		return resolved
			? { resolvedModule: resolved }
			: { resolvedModule: undefined, failedLookupLocations: failedLookupLocations(moduleName, compilerOptions ?? {}) };
	},
	flattenDiagnosticMessageText: diagnostics.flattenDiagnosticMessageText,
	formatDiagnostic: diagnostics.formatDiagnostic,
	formatDiagnostics: diagnostics.formatDiagnostics,
	formatDiagnosticsWithColorAndContext: diagnostics.formatDiagnosticsWithColorAndContext,
	sortAndDeduplicateDiagnostics: diagnostics.sortAndDeduplicateDiagnostics,
	diagnosticCategoryName: diagnostics.diagnosticCategoryName,
};

// Where tsc looked for an unresolved specifier, as far as tools read it: the
// candidates of the `paths` substitution it matched, a URL target kept as
// written (`tryLoadModuleUsingPaths`, extension by extension).
function failedLookupLocations(moduleName, options) {
	let matched;
	let matchedPrefix = -1;
	let captured = "";
	for (const [pattern, targets] of Object.entries(options.paths ?? {})) {
		const star = pattern.indexOf("*");
		if (star === -1) {
			if (pattern === moduleName) [matched, matchedPrefix, captured] = [targets, Infinity, ""];
			continue;
		}
		const prefix = pattern.slice(0, star);
		const suffix = pattern.slice(star + 1);
		if (
			prefix.length > matchedPrefix &&
			moduleName.length >= prefix.length + suffix.length &&
			moduleName.startsWith(prefix) &&
			moduleName.endsWith(suffix)
		) {
			[matched, matchedPrefix, captured] = [targets, prefix.length, moduleName.slice(prefix.length, moduleName.length - suffix.length)];
		}
	}
	if (!matched) return [];
	const base = options.baseUrl ?? options.pathsBasePath ?? process.cwd();
	return matched.flatMap((target) => {
		const substituted = target.replace("*", captured);
		const located = /^[a-z][a-z0-9+.-]*:\/\//i.test(substituted) ? substituted : require("node:path").resolve(base, substituted);
		return [".ts", ".tsx", ".d.ts"].map((extension) => located + extension);
	});
}

for (const [name, kinds] of Object.entries(guards)) {
	if (kinds.length === 1) {
		const [kind] = kinds;
		ts[name] = (node) => node.kind === kind;
	} else {
		const accepted = new Set(kinds);
		ts[name] = (node) => accepted.has(node.kind);
	}
}

// Helpers TypeScript keeps internal, provided because tools call them
// (docs/COMPILER_API.md, "TypeScript's internal API").
Object.assign(ts, require("./lib/paths.cjs"), require("./lib/utilities.cjs"), require("./lib/syntax.cjs"));
ts.parseIsolatedEntityName = require("./lib/internals.cjs").parseIsolatedEntityName;
require("./lib/tokens.cjs");
ts.FindAllReferences = require("./lib/references.cjs").FindAllReferences;
const messages = require("./lib/messages.cjs");
ts.formatMessage = messages.formatMessage;
ts.formatStringFromArgs = messages.formatStringFromArgs;
ts.getLocaleSpecificMessage = messages.getLocaleSpecificMessage;
ts.createCompilerDiagnostic = messages.createCompilerDiagnostic;
ts.createFileDiagnostic = messages.createFileDiagnostic;
Object.defineProperty(ts, "Diagnostics", { enumerable: true, get: messages.diagnostics });

module.exports = ts;
module.exports.default = ts;
