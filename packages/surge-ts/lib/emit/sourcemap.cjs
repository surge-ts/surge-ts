"use strict";

// tsc's `createSourceMapGenerator` (sourcemap.ts): mappings are committed one
// generated position at a time, the last source position written there
// winning unless it moves backwards, and serialized as base64 VLQs.
const { getRelativePathToDirectoryOrUrl } = require("./paths.cjs");

const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

function createSourceMapGenerator(host, file, sourceRoot, sourcesDirectoryPath) {
	const rawSources = [];
	const sources = [];
	const sourceToSourceIndexMap = new Map();
	const names = [];
	let mappings = "";
	let lastGeneratedLine = 0;
	let lastGeneratedCharacter = 0;
	let lastSourceIndex = 0;
	let lastSourceLine = 0;
	let lastSourceCharacter = 0;
	let lastNameIndex = 0;
	let hasLast = false;
	let pendingGeneratedLine = 0;
	let pendingGeneratedCharacter = 0;
	let pendingSourceIndex = 0;
	let pendingSourceLine = 0;
	let pendingSourceCharacter = 0;
	let pendingNameIndex = 0;
	let hasPending = false;
	let hasPendingSource = false;
	let hasPendingName = false;

	return {
		getSources: () => rawSources,
		addSource,
		addMapping,
		toJSON,
		toString: () => JSON.stringify(toJSON()),
	};

	function addSource(fileName) {
		const source = getRelativePathToDirectoryOrUrl(sourcesDirectoryPath, fileName, host.getCurrentDirectory(), host.getCanonicalFileName, true);
		let sourceIndex = sourceToSourceIndexMap.get(source);
		if (sourceIndex === undefined) {
			sourceIndex = sources.length;
			sources.push(source);
			rawSources.push(fileName);
			sourceToSourceIndexMap.set(source, sourceIndex);
		}
		return sourceIndex;
	}

	function isNewGeneratedPosition(generatedLine, generatedCharacter) {
		return !hasPending || pendingGeneratedLine !== generatedLine || pendingGeneratedCharacter !== generatedCharacter;
	}

	function isBacktrackingSourcePosition(sourceIndex, sourceLine, sourceCharacter) {
		return (
			sourceIndex !== undefined &&
			sourceLine !== undefined &&
			sourceCharacter !== undefined &&
			pendingSourceIndex === sourceIndex &&
			(pendingSourceLine > sourceLine || (pendingSourceLine === sourceLine && pendingSourceCharacter > sourceCharacter))
		);
	}

	function addMapping(generatedLine, generatedCharacter, sourceIndex, sourceLine, sourceCharacter, nameIndex) {
		if (isNewGeneratedPosition(generatedLine, generatedCharacter) || isBacktrackingSourcePosition(sourceIndex, sourceLine, sourceCharacter)) {
			commitPendingMapping();
			pendingGeneratedLine = generatedLine;
			pendingGeneratedCharacter = generatedCharacter;
			hasPendingSource = false;
			hasPendingName = false;
			hasPending = true;
		}
		if (sourceIndex !== undefined && sourceLine !== undefined && sourceCharacter !== undefined) {
			pendingSourceIndex = sourceIndex;
			pendingSourceLine = sourceLine;
			pendingSourceCharacter = sourceCharacter;
			hasPendingSource = true;
			if (nameIndex !== undefined) {
				pendingNameIndex = nameIndex;
				hasPendingName = true;
			}
		}
	}

	function shouldCommitMapping() {
		return (
			!hasLast ||
			lastGeneratedLine !== pendingGeneratedLine ||
			lastGeneratedCharacter !== pendingGeneratedCharacter ||
			lastSourceIndex !== pendingSourceIndex ||
			lastSourceLine !== pendingSourceLine ||
			lastSourceCharacter !== pendingSourceCharacter ||
			lastNameIndex !== pendingNameIndex
		);
	}

	function commitPendingMapping() {
		if (!hasPending || !shouldCommitMapping()) return;
		if (lastGeneratedLine < pendingGeneratedLine) {
			do {
				mappings += ";";
				lastGeneratedLine++;
			} while (lastGeneratedLine < pendingGeneratedLine);
			lastGeneratedCharacter = 0;
		} else if (hasLast) {
			mappings += ",";
		}
		appendBase64VLQ(pendingGeneratedCharacter - lastGeneratedCharacter);
		lastGeneratedCharacter = pendingGeneratedCharacter;
		if (hasPendingSource) {
			appendBase64VLQ(pendingSourceIndex - lastSourceIndex);
			lastSourceIndex = pendingSourceIndex;
			appendBase64VLQ(pendingSourceLine - lastSourceLine);
			lastSourceLine = pendingSourceLine;
			appendBase64VLQ(pendingSourceCharacter - lastSourceCharacter);
			lastSourceCharacter = pendingSourceCharacter;
			if (hasPendingName) {
				appendBase64VLQ(pendingNameIndex - lastNameIndex);
				lastNameIndex = pendingNameIndex;
			}
		}
		hasLast = true;
	}

	function toJSON() {
		commitPendingMapping();
		return { version: 3, file, sourceRoot, sources, names, mappings, sourcesContent: undefined };
	}

	function appendBase64VLQ(inValue) {
		inValue = inValue < 0 ? (-inValue << 1) + 1 : inValue << 1;
		do {
			let digit = inValue & 31;
			inValue >>= 5;
			if (inValue > 0) digit |= 32;
			mappings += BASE64[digit];
		} while (inValue > 0);
	}
}

module.exports = { createSourceMapGenerator };
