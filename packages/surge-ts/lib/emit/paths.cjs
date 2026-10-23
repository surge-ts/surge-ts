"use strict";

// Where tsc writes a file's declarations and their map, and how the map and
// the declaration file name each other (emitter.ts, path.ts, utilities.ts).
const paths = require("../paths.cjs");

const equateCaseSensitive = (a, b) => a === b;
const equateCaseInsensitive = (a, b) => a === b || a.toUpperCase() === b.toUpperCase();

/** tsc's `getPathComponentsRelativeTo`. */
function getPathComponentsRelativeTo(from, to, getCanonicalFileName) {
	const fromComponents = paths.reducePathComponents(paths.getPathComponents(from));
	const toComponents = paths.reducePathComponents(paths.getPathComponents(to));
	let start;
	for (start = 0; start < fromComponents.length && start < toComponents.length; start++) {
		const fromComponent = getCanonicalFileName(fromComponents[start]);
		const toComponent = getCanonicalFileName(toComponents[start]);
		const comparer = start === 0 ? equateCaseInsensitive : equateCaseSensitive;
		if (!comparer(fromComponent, toComponent)) break;
	}
	if (start === 0) return toComponents;
	const components = toComponents.slice(start);
	const relative = [];
	for (; start < fromComponents.length; start++) relative.push("..");
	return ["", ...relative, ...components];
}

/** tsc's `getRelativePathToDirectoryOrUrl`. */
function getRelativePathToDirectoryOrUrl(directoryPathOrUrl, relativeOrAbsolutePath, currentDirectory, getCanonicalFileName, isAbsolutePathAnUrl) {
	const components = getPathComponentsRelativeTo(
		paths.resolvePath(currentDirectory, directoryPathOrUrl),
		paths.resolvePath(currentDirectory, relativeOrAbsolutePath),
		getCanonicalFileName,
	);
	const first = components[0];
	if (isAbsolutePathAnUrl && paths.isRootedDiskPath(first)) {
		components[0] = (first.charAt(0) === "/" ? "file://" : "file:///") + first;
	}
	return paths.getPathFromPathComponents(components);
}

/** tsc's `getCommonSourceDirectory` (emitter.ts), for a program's emit. */
function getCommonSourceDirectory(program, getCanonicalFileName) {
	const emitted = () =>
		program
			.getSourceFiles()
			.filter((file) => !file.isDeclarationFile && !program.isSourceFileFromExternalLibrary(file) && !file.fileName.endsWith(".json"))
			.map((file) => file.fileName);
	return commonSourceDirectoryOf(program.getCompilerOptions(), emitted, program.getCurrentDirectory(), getCanonicalFileName);
}

/** tsc's `getCommonSourceDirectory` (program.ts) over the files a project emits. */
function commonSourceDirectoryOf(options, emittedFiles, currentDirectory, getCanonicalFileName) {
	let directory;
	if (options.rootDir) {
		directory = paths.getNormalizedAbsolutePath(options.rootDir, currentDirectory);
	} else if (options.composite && options.configFilePath) {
		directory = paths.getDirectoryPath(paths.normalizeSlashes(options.configFilePath));
	} else {
		directory = computeCommonSourceDirectoryOfFilenames(typeof emittedFiles === "function" ? emittedFiles() : emittedFiles, currentDirectory, getCanonicalFileName);
	}
	if (directory && directory[directory.length - 1] !== paths.directorySeparator) directory += paths.directorySeparator;
	return directory;
}

/** tsc's `computeCommonSourceDirectoryOfFilenames`. */
function computeCommonSourceDirectoryOfFilenames(fileNames, currentDirectory, getCanonicalFileName) {
	let commonPathComponents;
	for (const sourceFile of fileNames) {
		const components = paths.reducePathComponents(paths.getPathComponents(paths.getNormalizedAbsolutePath(sourceFile, currentDirectory)));
		components.pop();
		if (!commonPathComponents) {
			commonPathComponents = components;
			continue;
		}
		const n = Math.min(commonPathComponents.length, components.length);
		for (let i = 0; i < n; i++) {
			if (getCanonicalFileName(commonPathComponents[i]) !== getCanonicalFileName(components[i])) {
				if (i === 0) return "";
				commonPathComponents.length = i;
				break;
			}
		}
		if (components.length < commonPathComponents.length) commonPathComponents.length = components.length;
	}
	if (!commonPathComponents) return currentDirectory;
	return paths.getPathFromPathComponents(commonPathComponents);
}

/** tsc's `getSourceFilePathInNewDirWorker`. */
function getSourceFilePathInNewDir(fileName, newDirPath, currentDirectory, commonSourceDirectory, getCanonicalFileName) {
	let sourceFilePath = paths.getNormalizedAbsolutePath(fileName, currentDirectory);
	const inCommon = getCanonicalFileName(sourceFilePath).indexOf(getCanonicalFileName(commonSourceDirectory)) === 0;
	sourceFilePath = inCommon ? sourceFilePath.substring(commonSourceDirectory.length) : sourceFilePath;
	return paths.combinePaths(newDirPath, sourceFilePath);
}

/** tsc's `getDeclarationEmitExtensionForPath`. */
function getDeclarationEmitExtensionForPath(path) {
	const lower = path.toLowerCase();
	if (lower.endsWith(".mjs") || lower.endsWith(".mts")) return ".d.mts";
	if (lower.endsWith(".cjs") || lower.endsWith(".cts")) return ".d.cts";
	if (lower.endsWith(".json")) return ".d.json.ts";
	return ".d.ts";
}

/**
 * The declaration file and map tsc writes for `fileName`
 * (`getOutputPathsFor` with `forceDtsPaths`).
 */
function getDeclarationOutputPaths(fileName, options, host) {
	const outputDir = options.declarationDir || options.outDir;
	const path = outputDir
		? getSourceFilePathInNewDir(fileName, outputDir, host.getCurrentDirectory(), host.getCommonSourceDirectory(), host.getCanonicalFileName)
		: fileName;
	const declarationFilePath = paths.removeFileExtension(path) + getDeclarationEmitExtensionForPath(path);
	const declarationMapPath = options.declarationMap ? `${declarationFilePath}.map` : undefined;
	return { declarationFilePath, declarationMapPath };
}

/** tsc's `getSourceRoot`. */
function getSourceRoot(options) {
	const sourceRoot = paths.normalizeSlashes(options.sourceRoot || "");
	return sourceRoot ? paths.ensureTrailingDirectorySeparator(sourceRoot) : sourceRoot;
}

/** tsc's `getSourceMapDirectory`. */
function getSourceMapDirectory(options, filePath, sourceFile, host) {
	if (options.sourceRoot) return host.getCommonSourceDirectory();
	if (options.mapRoot) {
		let sourceMapDir = paths.normalizeSlashes(options.mapRoot);
		if (sourceFile) {
			sourceMapDir = paths.getDirectoryPath(
				getSourceFilePathInNewDir(sourceFile.fileName, sourceMapDir, host.getCurrentDirectory(), host.getCommonSourceDirectory(), host.getCanonicalFileName),
			);
		}
		if (paths.getRootLength(sourceMapDir) === 0) sourceMapDir = paths.combinePaths(host.getCommonSourceDirectory(), sourceMapDir);
		return sourceMapDir;
	}
	return paths.getDirectoryPath(paths.normalizePath(filePath));
}

/** tsc's `getSourceMappingURL`. */
function getSourceMappingURL(options, filePath, sourceMapFilePath, sourceFile, host) {
	const sourceMapFile = paths.getBaseFileName(paths.normalizeSlashes(sourceMapFilePath));
	if (options.mapRoot) {
		let sourceMapDir = paths.normalizeSlashes(options.mapRoot);
		if (sourceFile) {
			sourceMapDir = paths.getDirectoryPath(
				getSourceFilePathInNewDir(sourceFile.fileName, sourceMapDir, host.getCurrentDirectory(), host.getCommonSourceDirectory(), host.getCanonicalFileName),
			);
		}
		if (paths.getRootLength(sourceMapDir) === 0) {
			sourceMapDir = paths.combinePaths(host.getCommonSourceDirectory(), sourceMapDir);
			return encodeURI(
				getRelativePathToDirectoryOrUrl(
					paths.getDirectoryPath(paths.normalizePath(filePath)),
					paths.combinePaths(sourceMapDir, sourceMapFile),
					host.getCurrentDirectory(),
					host.getCanonicalFileName,
					true,
				),
			);
		}
		return encodeURI(paths.combinePaths(sourceMapDir, sourceMapFile));
	}
	return encodeURI(sourceMapFile);
}

module.exports = {
	getRelativePathToDirectoryOrUrl,
	getCommonSourceDirectory,
	commonSourceDirectoryOf,
	getSourceFilePathInNewDir,
	getDeclarationOutputPaths,
	getSourceRoot,
	getSourceMapDirectory,
	getSourceMappingURL,
};
