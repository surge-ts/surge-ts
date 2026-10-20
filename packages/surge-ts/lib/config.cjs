"use strict";

// `readConfigFile`, `parseConfigFileTextToJson`, `parseJsonConfigFileContent`,
// `findConfigFile`, `getParsedCommandLineOfConfigFile`: the config parsing,
// `extends` merging, option normalization and file discovery are the native
// ones the `surge` CLI uses.
const path = require("node:path");
const native = require("./native.cjs");
const { DiagnosticCategory } = require("./enums.cjs");
const { sys, toSlash } = require("./sys.cjs");

function configDiagnostic(record) {
	const diagnostic = {
		file: undefined,
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

function parseConfigFileTextToJson(fileName, jsonText) {
	const result = native.parseConfigFileText(fileName, jsonText);
	return result.error ? { config: {}, error: configDiagnostic(result.error) } : { config: result.config };
}

function readConfigFile(fileName, readFile) {
	const text = readFile(fileName);
	if (text === undefined) {
		return {
			config: {},
			error: {
				file: undefined,
				start: undefined,
				length: undefined,
				code: 5083,
				category: DiagnosticCategory.Error,
				messageText: `Cannot read file '${fileName}'.`,
			},
		};
	}
	return parseConfigFileTextToJson(fileName, text);
}

function isSystemParseHost(host) {
	return host === undefined || host === sys || (host.readDirectory === sys.readDirectory && host.readFile === sys.readFile);
}

/**
 * `parseJsonConfigFileContent`. Files and `extends` are read from the file
 * system; a host that reads elsewhere is not supported yet.
 */
function parseJsonConfigFileContent(json, host, basePath, existingOptions, configFileName) {
	if (!isSystemParseHost(host)) {
		throw new Error("surge-ts: parseJsonConfigFileContent reads the file system; a custom ParseConfigHost is not supported yet");
	}
	const parsed = native.parseJsonConfigFileContent(
		json ?? {},
		toSlash(path.resolve(basePath)),
		existingOptions ? JSON.parse(JSON.stringify(existingOptions)) : undefined,
		configFileName ? toSlash(path.resolve(basePath, configFileName)) : undefined,
	);
	return {
		options: parsed.options,
		fileNames: parsed.fileNames,
		errors: parsed.errors.map(configDiagnostic),
		raw: json,
		projectReferences: projectReferencesOf(json, basePath),
		typeAcquisition: undefined,
		wildcardDirectories: undefined,
		compileOnSave: undefined,
	};
}

/**
 * tsc's `getProjectReferences`: the config's own `references` (`extends`
 * passes none on), each path made absolute against the config's directory.
 */
function projectReferencesOf(json, basePath) {
	if (!Array.isArray(json?.references)) return undefined;
	const references = json.references
		.filter((reference) => typeof reference === "object" && reference !== null && typeof reference.path === "string")
		.map((reference) => ({
			path: toSlash(path.resolve(basePath, reference.path)),
			originalPath: reference.path,
			prepend: reference.prepend,
			circular: reference.circular,
		}));
	return references.length > 0 ? references : undefined;
}

/** `findConfigFile`: the nearest `configName` at or above `searchPath`. */
function findConfigFile(searchPath, fileExists, configName = "tsconfig.json") {
	let directory = toSlash(path.resolve(searchPath));
	for (;;) {
		const candidate = `${directory === "/" ? "" : directory}/${configName}`;
		if (fileExists(candidate)) return candidate;
		const parent = toSlash(path.dirname(directory));
		if (parent === directory) return undefined;
		directory = parent;
	}
}

/** `getParsedCommandLineOfConfigFile`. */
function getParsedCommandLineOfConfigFile(configFileName, optionsToExtend, host) {
	const readFile = host?.readFile ? (fileName) => host.readFile(fileName) : sys.readFile;
	const { config, error } = readConfigFile(configFileName, readFile);
	if (error) {
		host?.onUnRecoverableConfigFileDiagnostic?.(error);
		return undefined;
	}
	const absolute = toSlash(path.resolve(host?.getCurrentDirectory?.() ?? process.cwd(), configFileName));
	return parseJsonConfigFileContent(config, sys, path.dirname(absolute), optionsToExtend, absolute);
}

module.exports = { readConfigFile, parseConfigFileTextToJson, parseJsonConfigFileContent, findConfigFile, getParsedCommandLineOfConfigFile };
