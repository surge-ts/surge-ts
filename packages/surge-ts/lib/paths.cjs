"use strict";

// TypeScript's path helpers (src/compiler/path.ts), ported. They are
// internal to TypeScript; surge-ts provides them because tools built on its
// compiler API call them.

const directorySeparator = "/";
const backslashRegExp = /\\/g;
const relativePathSegmentRegExp = /\/\/|(?:^|\/)\.\.?(?:$|\/)/;
const fileNameLowerCaseRegExp = /[^İıßa-z0-9\\/:\-_. ]+/g;
const urlSchemeSeparator = "://";

function isAnyDirectorySeparator(charCode) {
	return charCode === 47 || charCode === 92;
}

function isVolumeCharacter(charCode) {
	return (charCode >= 97 && charCode <= 122) || (charCode >= 65 && charCode <= 90);
}

function getFileUrlVolumeSeparatorEnd(url, start) {
	const ch0 = url.charCodeAt(start);
	if (ch0 === 58) return start + 1;
	if (ch0 === 37 && url.charCodeAt(start + 1) === 51) {
		const ch2 = url.charCodeAt(start + 2);
		if (ch2 === 97 || ch2 === 65) return start + 3;
	}
	return -1;
}

/** The root's length, or its bitwise complement for a URL. */
function getEncodedRootLength(path) {
	if (!path) return 0;
	const ch0 = path.charCodeAt(0);
	if (ch0 === 47 || ch0 === 92) {
		if (path.charCodeAt(1) !== ch0) return 1;
		const p1 = path.indexOf(ch0 === 47 ? directorySeparator : "\\", 2);
		if (p1 < 0) return path.length;
		return p1 + 1;
	}
	if (isVolumeCharacter(ch0) && path.charCodeAt(1) === 58) {
		const ch2 = path.charCodeAt(2);
		if (ch2 === 47 || ch2 === 92) return 3;
		if (path.length === 2) return 2;
	}
	const schemeEnd = path.indexOf(urlSchemeSeparator);
	if (schemeEnd !== -1) {
		const authorityStart = schemeEnd + urlSchemeSeparator.length;
		const authorityEnd = path.indexOf(directorySeparator, authorityStart);
		if (authorityEnd !== -1) {
			const scheme = path.slice(0, schemeEnd);
			const authority = path.slice(authorityStart, authorityEnd);
			if (scheme === "file" && (authority === "" || authority === "localhost") && isVolumeCharacter(path.charCodeAt(authorityEnd + 1))) {
				const volumeSeparatorEnd = getFileUrlVolumeSeparatorEnd(path, authorityEnd + 2);
				if (volumeSeparatorEnd !== -1) {
					if (path.charCodeAt(volumeSeparatorEnd) === 47) return ~(volumeSeparatorEnd + 1);
					if (volumeSeparatorEnd === path.length) return ~volumeSeparatorEnd;
				}
			}
			return ~(authorityEnd + 1);
		}
		return ~path.length;
	}
	return 0;
}

function getRootLength(path) {
	const rootLength = getEncodedRootLength(path);
	return rootLength < 0 ? ~rootLength : rootLength;
}

function isRootedDiskPath(path) {
	return getEncodedRootLength(path) > 0;
}

function normalizeSlashes(path) {
	return path.includes("\\") ? path.replace(backslashRegExp, directorySeparator) : path;
}

function hasTrailingDirectorySeparator(path) {
	return path.length > 0 && isAnyDirectorySeparator(path.charCodeAt(path.length - 1));
}

function ensureTrailingDirectorySeparator(path) {
	return hasTrailingDirectorySeparator(path) ? path : path + directorySeparator;
}

function removeTrailingDirectorySeparator(path) {
	return hasTrailingDirectorySeparator(path) ? path.slice(0, -1) : path;
}

function combinePaths(path, ...paths) {
	if (path) path = normalizeSlashes(path);
	for (let relativePath of paths) {
		if (!relativePath) continue;
		relativePath = normalizeSlashes(relativePath);
		path = !path || getRootLength(relativePath) !== 0 ? relativePath : ensureTrailingDirectorySeparator(path) + relativePath;
	}
	return path;
}

/** `undefined` when the path still has `.`/`..` segments or doubled slashes. */
function simpleNormalizePath(path) {
	if (!relativePathSegmentRegExp.test(path)) return path;
	let simplified = path.replace(/\/\.\//g, "/");
	if (simplified.startsWith("./")) simplified = simplified.slice(2);
	if (simplified !== path) {
		path = simplified;
		if (!relativePathSegmentRegExp.test(path)) return path;
	}
	return undefined;
}

function getNormalizedAbsolutePath(path, currentDirectory) {
	let rootLength = getRootLength(path);
	if (rootLength === 0 && currentDirectory) {
		path = combinePaths(currentDirectory, path);
		rootLength = getRootLength(path);
	} else {
		path = normalizeSlashes(path);
	}
	const simpleNormalized = simpleNormalizePath(path);
	if (simpleNormalized !== undefined) {
		return simpleNormalized.length > rootLength ? removeTrailingDirectorySeparator(simpleNormalized) : simpleNormalized;
	}
	const length = path.length;
	const root = path.substring(0, rootLength);
	let normalized;
	let index = rootLength;
	let segmentStart = index;
	let normalizedUpTo = index;
	let seenNonDotDotSegment = rootLength !== 0;
	while (index < length) {
		segmentStart = index;
		let ch = path.charCodeAt(index);
		while (ch === 47 && index + 1 < length) {
			index++;
			ch = path.charCodeAt(index);
		}
		if (index > segmentStart) {
			normalized ??= path.substring(0, segmentStart - 1);
			segmentStart = index;
		}
		let segmentEnd = path.indexOf(directorySeparator, index + 1);
		if (segmentEnd === -1) segmentEnd = length;
		const segmentLength = segmentEnd - segmentStart;
		if (segmentLength === 1 && path.charCodeAt(index) === 46) {
			normalized ??= path.substring(0, normalizedUpTo);
		} else if (segmentLength === 2 && path.charCodeAt(index) === 46 && path.charCodeAt(index + 1) === 46) {
			if (!seenNonDotDotSegment) {
				if (normalized !== undefined) normalized += normalized.length === rootLength ? ".." : "/..";
				else normalizedUpTo = index + 2;
			} else if (normalized === undefined) {
				normalized =
					normalizedUpTo - 2 >= 0
						? path.substring(0, Math.max(rootLength, path.lastIndexOf(directorySeparator, normalizedUpTo - 2)))
						: path.substring(0, normalizedUpTo);
			} else {
				const lastSlash = normalized.lastIndexOf(directorySeparator);
				normalized = lastSlash !== -1 ? normalized.substring(0, Math.max(rootLength, lastSlash)) : root;
				if (normalized.length === rootLength) seenNonDotDotSegment = rootLength !== 0;
			}
		} else if (normalized !== undefined) {
			if (normalized.length !== rootLength) normalized += directorySeparator;
			seenNonDotDotSegment = true;
			normalized += path.substring(segmentStart, segmentEnd);
		} else {
			seenNonDotDotSegment = true;
			normalizedUpTo = segmentEnd;
		}
		index = segmentEnd + 1;
	}
	return normalized ?? (length > rootLength ? removeTrailingDirectorySeparator(path) : path);
}

function normalizePath(path) {
	path = normalizeSlashes(path);
	const normalized = simpleNormalizePath(path);
	if (normalized !== undefined) return normalized;
	const absolute = getNormalizedAbsolutePath(path, "");
	return absolute && hasTrailingDirectorySeparator(path) ? ensureTrailingDirectorySeparator(absolute) : absolute;
}

function resolvePath(path, ...paths) {
	return normalizePath(paths.length > 0 ? combinePaths(path, ...paths) : normalizeSlashes(path));
}

function toPath(fileName, basePath, getCanonicalFileName) {
	const nonCanonicalized = isRootedDiskPath(fileName) ? normalizePath(fileName) : getNormalizedAbsolutePath(fileName, basePath);
	return getCanonicalFileName(nonCanonicalized);
}

function toFileNameLowerCase(fileName) {
	return fileNameLowerCaseRegExp.test(fileName) ? fileName.replace(fileNameLowerCaseRegExp, (text) => text.toLowerCase()) : fileName;
}

function createGetCanonicalFileName(useCaseSensitiveFileNames) {
	return useCaseSensitiveFileNames ? (fileName) => fileName : toFileNameLowerCase;
}

function getDirectoryPath(path) {
	path = normalizeSlashes(path);
	const rootLength = getRootLength(path);
	if (rootLength === path.length) return path;
	path = removeTrailingDirectorySeparator(path);
	return path.slice(0, Math.max(rootLength, path.lastIndexOf(directorySeparator)));
}

function getBaseFileName(path) {
	path = normalizeSlashes(path);
	const rootLength = getRootLength(path);
	if (rootLength === path.length) return "";
	path = removeTrailingDirectorySeparator(path);
	return path.slice(Math.max(getRootLength(path), path.lastIndexOf(directorySeparator) + 1));
}

function getPathComponents(path, currentDirectory = "") {
	path = combinePaths(currentDirectory, path);
	const rootLength = getRootLength(path);
	const rest = path.substring(rootLength).split(directorySeparator);
	if (rest.length && !rest[rest.length - 1]) rest.pop();
	return [path.substring(0, rootLength), ...rest];
}

function reducePathComponents(components) {
	if (components.length === 0) return [];
	const reduced = [components[0]];
	for (let index = 1; index < components.length; index++) {
		const component = components[index];
		if (!component || component === ".") continue;
		if (component === "..") {
			if (reduced.length > 1) {
				if (reduced[reduced.length - 1] !== "..") {
					reduced.pop();
					continue;
				}
			} else if (reduced[0]) {
				continue;
			}
		}
		reduced.push(component);
	}
	return reduced;
}

function getPathFromPathComponents(components, length) {
	if (components.length === 0) return "";
	const root = components[0] && ensureTrailingDirectorySeparator(components[0]);
	return root + components.slice(1, length).join(directorySeparator);
}

function getRelativePathFromDirectory(fromDirectory, to, getCanonicalFileNameOrIgnoreCase) {
	const getCanonicalFileName = typeof getCanonicalFileNameOrIgnoreCase === "function" ? getCanonicalFileNameOrIgnoreCase : (name) => name;
	const ignoreCase = typeof getCanonicalFileNameOrIgnoreCase === "boolean" ? getCanonicalFileNameOrIgnoreCase : false;
	const fromComponents = reducePathComponents(getPathComponents(fromDirectory));
	const toComponents = reducePathComponents(getPathComponents(to));
	let start;
	for (start = 0; start < fromComponents.length && start < toComponents.length; start++) {
		const from = getCanonicalFileName(fromComponents[start]);
		const target = getCanonicalFileName(toComponents[start]);
		const equal = start === 0 || ignoreCase ? from.toUpperCase() === target.toUpperCase() : from === target;
		if (!equal) break;
	}
	if (start === 0) return getPathFromPathComponents(toComponents);
	const relative = [];
	for (let index = start; index < fromComponents.length; index++) relative.push("..");
	return getPathFromPathComponents(["", ...relative, ...toComponents.slice(start)]);
}

function fileExtensionIs(path, extension) {
	return path.length > extension.length && path.endsWith(extension);
}

const declarationExtensions = [".d.ts", ".d.cts", ".d.mts"];

function getDeclarationFileExtension(fileName) {
	const standard = declarationExtensions.find((extension) => fileName.endsWith(extension));
	if (standard) return standard;
	if (fileExtensionIs(fileName, ".ts")) {
		const baseName = getBaseFileName(fileName);
		const index = baseName.lastIndexOf(".d.");
		if (index >= 0) return baseName.substring(index);
	}
	return undefined;
}

function isDeclarationFileName(fileName) {
	return getDeclarationFileExtension(fileName) !== undefined;
}

const extensionsToRemove = [".d.ts", ".d.mts", ".d.cts", ".mjs", ".mts", ".cjs", ".cts", ".ts", ".js", ".tsx", ".jsx", ".json"];

function removeFileExtension(path) {
	const extension = extensionsToRemove.find((candidate) => fileExtensionIs(path, candidate));
	return extension ? path.slice(0, path.length - extension.length) : path;
}

function isInsideNodeModules(fileOrDirectory) {
	return getPathComponents(fileOrDirectory).includes("node_modules");
}

function forEachAncestorDirectory(directory, callback) {
	while (true) {
		const result = callback(directory);
		if (result !== undefined) return result;
		const parent = getDirectoryPath(directory);
		if (parent === directory) return undefined;
		directory = parent;
	}
}

/** The nearest `package.json` at or above `directory`, short of a `node_modules` folder. */
function findPackageJson(directory, host) {
	const globalCache = host?.getGlobalTypingsCacheLocation?.();
	let packageJson;
	forEachAncestorDirectory(directory, (ancestor) => {
		if (ancestor === "node_modules") return true;
		const candidate = combinePaths(ancestor, "package.json");
		if (host.fileExists(candidate)) {
			packageJson = candidate;
			return true;
		}
		if (ancestor === globalCache) return false;
	});
	return packageJson;
}

module.exports = {
	directorySeparator,
	isAnyDirectorySeparator,
	getRootLength,
	isRootedDiskPath,
	normalizeSlashes,
	hasTrailingDirectorySeparator,
	ensureTrailingDirectorySeparator,
	removeTrailingDirectorySeparator,
	combinePaths,
	getNormalizedAbsolutePath,
	normalizePath,
	resolvePath,
	toPath,
	toFileNameLowerCase,
	createGetCanonicalFileName,
	getDirectoryPath,
	getBaseFileName,
	getPathComponents,
	reducePathComponents,
	getPathFromPathComponents,
	getRelativePathFromDirectory,
	fileExtensionIs,
	isDeclarationFileName,
	removeFileExtension,
	isInsideNodeModules,
	forEachAncestorDirectory,
	findPackageJson,
};
