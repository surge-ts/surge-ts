"use strict";

// tsc's printer (emitter.ts `createPrinter`) for what a declaration file
// holds, as declaration emit configures it: only JSDoc-style comments, no
// source positions for braces. A node with a text range writes its source
// position before and after itself into the source map; a node made by the
// transform writes none, which is how the map follows tsc's exactly.
const { SyntaxKind: K, NodeFlags } = require("../enums.cjs");
const { tokenToString } = require("../syntax.cjs");
const f = require("./factory.cjs");
const { computeLineStarts } = require("./writer.cjs");
const { phaseModifierOf } = require("./declarations.cjs");

const EmitFlags = f.EmitFlags;
const { parenthesizer } = f;

const ListFormat = {
	None: 0,
	SingleLine: 0,
	MultiLine: 1,
	PreserveLines: 2,
	LinesMask: 3,
	BarDelimited: 4,
	AmpersandDelimited: 8,
	CommaDelimited: 16,
	AsteriskDelimited: 32,
	DelimitersMask: 60,
	AllowTrailingComma: 64,
	Indented: 128,
	SpaceBetweenBraces: 256,
	SpaceBetweenSiblings: 512,
	Braces: 1024,
	Parenthesis: 2048,
	AngleBrackets: 4096,
	SquareBrackets: 8192,
	BracketsMask: 15360,
	OptionalIfUndefined: 16384,
	OptionalIfEmpty: 32768,
	PreferNewLine: 65536,
	NoTrailingNewLine: 131072,
	NoInterveningComments: 262144,
	NoSpaceIfEmpty: 524288,
	SpaceAfterList: 2097152,
	Modifiers: 2359808,
	HeritageClauses: 512,
	SingleLineTypeLiteralMembers: 768,
	MultiLineTypeLiteralMembers: 32897,
	SingleLineTupleTypeElements: 528,
	MultiLineTupleTypeElements: 657,
	UnionTypeConstituents: 516,
	IntersectionTypeConstituents: 520,
	ObjectBindingPatternElements: 525136,
	ArrayBindingPatternElements: 524880,
	ImportAttributes: 526226,
	TemplateExpressionSpans: 262144,
	SingleLineBlockStatements: 768,
	MultiLineBlockStatements: 129,
	VariableDeclarationList: 528,
	ClassHeritageClauses: 0,
	ClassMembers: 129,
	InterfaceMembers: 129,
	EnumMembers: 145,
	NamedImportsOrExportsElements: 525136,
	HeritageClauseTypes: 528,
	TypeArguments: 53776,
	TypeParameters: 53776,
	Parameters: 2576,
	IndexSignatureParameters: 8848,
};

const isLineBreak = (ch) => ch === 10 || ch === 13 || ch === 0x2028 || ch === 0x2029;
const isWhiteSpaceSingleLine = (ch) =>
	ch === 32 ||
	ch === 9 ||
	ch === 11 ||
	ch === 12 ||
	ch === 160 ||
	ch === 133 ||
	ch === 5760 ||
	(ch >= 8192 && ch <= 8203) ||
	ch === 8239 ||
	ch === 8287 ||
	ch === 12288 ||
	ch === 65279;
const isWhiteSpaceLike = (ch) => isWhiteSpaceSingleLine(ch) || isLineBreak(ch);

/** tsc's `skipTrivia`, without conflict markers. */
function skipTrivia(text, pos, stopAfterLineBreak, stopAtComments) {
	if (pos < 0) return pos;
	for (;;) {
		const ch = text.charCodeAt(pos);
		switch (ch) {
			case 13:
				if (text.charCodeAt(pos + 1) === 10) pos++;
			// falls through
			case 10:
				pos++;
				if (stopAfterLineBreak) return pos;
				continue;
			case 9:
			case 11:
			case 12:
			case 32:
				pos++;
				continue;
			case 47:
				if (stopAtComments) break;
				if (text.charCodeAt(pos + 1) === 47) {
					pos += 2;
					while (pos < text.length && !isLineBreak(text.charCodeAt(pos))) pos++;
					continue;
				}
				if (text.charCodeAt(pos + 1) === 42) {
					pos += 2;
					while (pos < text.length) {
						if (text.charCodeAt(pos) === 42 && text.charCodeAt(pos + 1) === 47) {
							pos += 2;
							break;
						}
						pos++;
					}
					continue;
				}
				break;
			case 35:
				if (pos === 0 && text.charCodeAt(1) === 33) {
					while (pos < text.length && !isLineBreak(text.charCodeAt(pos))) pos++;
					continue;
				}
				break;
			default:
				if (ch > 127 && isWhiteSpaceLike(ch)) {
					pos++;
					continue;
				}
		}
		return pos;
	}
}

/** tsc's `iterateCommentRanges`. */
function iterateCommentRanges(text, pos, trailing, cb, state) {
	let pendingPos;
	let pendingEnd;
	let pendingKind;
	let pendingHasTrailingNewLine;
	let hasPendingCommentRange = false;
	let collecting = trailing;
	if (pos === 0) {
		collecting = true;
		if (text.charCodeAt(0) === 35 && text.charCodeAt(1) === 33) {
			while (pos < text.length && !isLineBreak(text.charCodeAt(pos))) pos++;
		}
	}
	scan: while (pos >= 0 && pos < text.length) {
		const ch = text.charCodeAt(pos);
		switch (ch) {
			case 13:
				if (text.charCodeAt(pos + 1) === 10) pos++;
			// falls through
			case 10:
				pos++;
				if (trailing) break scan;
				collecting = true;
				if (hasPendingCommentRange) pendingHasTrailingNewLine = true;
				continue;
			case 9:
			case 11:
			case 12:
			case 32:
				pos++;
				continue;
			case 47: {
				const nextChar = text.charCodeAt(pos + 1);
				let hasTrailingNewLine = false;
				if (nextChar === 47 || nextChar === 42) {
					const kind = nextChar === 47 ? K.SingleLineCommentTrivia : K.MultiLineCommentTrivia;
					const startPos = pos;
					pos += 2;
					if (nextChar === 47) {
						while (pos < text.length) {
							if (isLineBreak(text.charCodeAt(pos))) {
								hasTrailingNewLine = true;
								break;
							}
							pos++;
						}
					} else {
						while (pos < text.length) {
							if (text.charCodeAt(pos) === 42 && text.charCodeAt(pos + 1) === 47) {
								pos += 2;
								break;
							}
							pos++;
						}
					}
					if (collecting) {
						if (hasPendingCommentRange) cb(pendingPos, pendingEnd, pendingKind, pendingHasTrailingNewLine, state);
						pendingPos = startPos;
						pendingEnd = pos;
						pendingKind = kind;
						pendingHasTrailingNewLine = hasTrailingNewLine;
						hasPendingCommentRange = true;
					}
					continue;
				}
				break scan;
			}
			default:
				if (ch > 127 && isWhiteSpaceLike(ch)) {
					if (hasPendingCommentRange && isLineBreak(ch)) pendingHasTrailingNewLine = true;
					pos++;
					continue;
				}
				break scan;
		}
	}
	if (hasPendingCommentRange) cb(pendingPos, pendingEnd, pendingKind, pendingHasTrailingNewLine, state);
}
const forEachLeadingCommentRange = (text, pos, cb, state) => iterateCommentRanges(text, pos, false, cb, state);
const forEachTrailingCommentRange = (text, pos, cb, state) => iterateCommentRanges(text, pos, true, cb, state);
function getLeadingCommentRanges(text, pos) {
	const ranges = [];
	forEachLeadingCommentRange(text, pos, (p, e, kind, hasTrailingNewLine) => ranges.push({ pos: p, end: e, kind, hasTrailingNewLine }));
	return ranges.length ? ranges : undefined;
}

function computeLineOfPosition(lineStarts, position) {
	let low = 0;
	let high = lineStarts.length - 1;
	while (low <= high) {
		const middle = (low + high) >>> 1;
		if (lineStarts[middle] === position) return middle;
		if (lineStarts[middle] < position) low = middle + 1;
		else high = middle - 1;
	}
	return low - 1;
}
function computeLineAndCharacterOfPosition(lineStarts, position) {
	const line = computeLineOfPosition(lineStarts, position);
	return { line, character: position - lineStarts[line] };
}

const isJSDocLikeText = (text, start) => text.charCodeAt(start + 1) === 42 && text.charCodeAt(start + 2) === 42 && text.charCodeAt(start + 3) !== 47;
const isPinnedComment = (text, start) => text.charCodeAt(start + 1) === 42 && text.charCodeAt(start + 2) === 33;
const TRIPLE_SLASH = [
	/^\/\/\/\s*<reference\s+path\s*=\s*('|")(.+?)\1.*?\/>/,
	/^\/\/\/\s*<amd-dependency\s+path\s*=\s*('|")(.+?)\1.*?\/>/,
	/^\/\/\/\s*<amd-module\s+.*?\/>/,
	/^\/\/\/\s*<reference\s+types\s*=\s*('|")(.+?)\1.*?\/>/,
	/^\/\/\/\s*<reference\s+lib\s*=\s*('|")(.+?)\1.*?\/>/,
	/^(\/\/\/\s*<reference\s+no-default-lib\s*=\s*)('|")(.+?)\2\s*\/>/,
];
function isRecognizedTripleSlashComment(text, commentPos, commentEnd) {
	if (text.charCodeAt(commentPos + 1) === 47 && commentPos + 2 < commentEnd && text.charCodeAt(commentPos + 2) === 47) {
		const textSubStr = text.substring(commentPos, commentEnd);
		return TRIPLE_SLASH.some((pattern) => pattern.test(textSubStr));
	}
	return false;
}

const ESCAPED = new Map([
	["\t", "\\t"],
	["\v", "\\v"],
	["\f", "\\f"],
	["\b", "\\b"],
	["\r", "\\r"],
	["\n", "\\n"],
	["\\", "\\\\"],
	['"', '\\"'],
	["'", "\\'"],
	["`", "\\`"],
	["\u2028", "\\u2028"],
	["\u2029", "\\u2029"],
	["\u0085", "\\u0085"],
	["\r\n", "\\r\\n"],
]);
const DOUBLE_QUOTE_ESCAPED = /[\\"\u0000-\u001f\t\v\f\b\r\n\u2028\u2029\u0085]/g;
const SINGLE_QUOTE_ESCAPED = /[\\'\u0000-\u001f\t\v\f\b\r\n\u2028\u2029\u0085]/g;
const BACKTICK_ESCAPED = /\r\n|[\\`\u0000-\u0009\u000b-\u001f\u2028\u2029\u0085]/g;
const NON_ASCII = /[^\u0000-\u007F]/g;
const encodeUtf16EscapeSequence = (charCode) => `\\u${`0000${charCode.toString(16).toUpperCase()}`.slice(-4)}`;
function getReplacement(c, offset, input) {
	if (c.charCodeAt(0) === 0) {
		const lookAhead = input.charCodeAt(offset + c.length);
		return lookAhead >= 48 && lookAhead <= 57 ? "\\x00" : "\\0";
	}
	return ESCAPED.get(c) || encodeUtf16EscapeSequence(c.charCodeAt(0));
}
function escapeString(s, quoteChar) {
	const pattern = quoteChar === 96 ? BACKTICK_ESCAPED : quoteChar === 39 ? SINGLE_QUOTE_ESCAPED : DOUBLE_QUOTE_ESCAPED;
	return s.replace(pattern, getReplacement);
}
function escapeNonAsciiString(s, quoteChar) {
	s = escapeString(s, quoteChar);
	return s.replace(NON_ASCII, (c) => encodeUtf16EscapeSequence(c.charCodeAt(0)));
}
const unescapeLeadingUnderscores = (name) =>
	name.length >= 3 && name.charCodeAt(0) === 95 && name.charCodeAt(1) === 95 && name.charCodeAt(2) === 95 ? name.slice(1) : name;
const idText = (node) => unescapeLeadingUnderscores(node.escapedText);

function isKeyword(kind) {
	return kind >= K.FirstKeyword && kind <= K.LastKeyword;
}
function isTokenKind(kind) {
	return kind >= K.FirstToken && kind <= K.LastToken;
}

/** The parse-tree source file a node of the original tree belongs to. */
function getSourceFileOfNode(node) {
	while (node && node.kind !== K.SourceFile) node = node.parent;
	return node;
}

function createPrinter(printerOptions = {}) {
	const omitBraceSourcePositions = !!printerOptions.omitBraceSourceMapPositions;
	const newLine = printerOptions.newLine === 0 ? "\r\n" : "\n";
	let currentSourceFile;
	let currentLineMap;
	let writer;
	let sourceMapsDisabled = true;
	let sourceMapGenerator;
	let sourceMapSource;
	let sourceMapSourceIndex = -1;
	let containerPos = -1;
	let containerEnd = -1;
	let declarationListContainerEnd = -1;
	let detachedCommentsInfo;
	let hasWrittenComment = false;
	let commentsDisabled = !!printerOptions.removeComments;
	let nextListElementPos;
	let write = (s) => writer.write(s);

	const writeKeyword = (s) => writer.writeKeyword(s);
	const writePunctuation = (s) => writer.writePunctuation(s);
	const writeOperator = (s) => writer.writeOperator(s);
	const writeSpace = () => writer.writeSpace(" ");
	const writeTrailingSemicolon = () => writer.writeTrailingSemicolon(";");
	const increaseIndent = () => writer.increaseIndent();
	const decreaseIndent = () => writer.decreaseIndent();

	return { writeFile };

	function writeFile(sourceFile, output, generator) {
		writer = output;
		sourceMapGenerator = generator;
		sourceMapsDisabled = !writer || !sourceMapGenerator;
		currentSourceFile = sourceFile;
		currentLineMap = undefined;
		detachedCommentsInfo = undefined;
		if (sourceFile.text.startsWith("#!")) {
			const end = sourceFile.text.search(/[\r\n\u2028\u2029]|$/);
			writer.writeComment(sourceFile.text.slice(0, end));
			writer.writeLine();
		}
		setSourceMapSource(sourceFile);
		pipelineEmit(0, sourceFile);
		currentSourceFile = undefined;
	}

	function getCurrentLineMap() {
		return (currentLineMap ??= computeLineStarts(currentSourceFile.text));
	}

	function setSourceMapSource(source) {
		if (sourceMapsDisabled) return;
		sourceMapSource = { fileName: source.fileName, text: source.text, lineMap: computeLineStarts(source.text) };
		sourceMapSourceIndex = sourceMapGenerator.addSource(source.fileName);
	}

	// --- pipeline ---------------------------------------------------------------

	function emit(node, parenthesizerRule) {
		if (node === undefined) return;
		pipelineEmit(4, node, parenthesizerRule);
	}
	function emitExpression(node, parenthesizerRule) {
		if (node === undefined) return;
		pipelineEmit(1, node, parenthesizerRule);
	}
	function emitIdentifierName(node) {
		if (node === undefined) return;
		pipelineEmit(2, node);
	}

	// tsc applies a list's parenthesizer rule only to a substituted node, and
	// declaration emit substitutes none: nodes come parenthesized from the
	// factory or the source.
	function pipelineEmit(hint, node) {
		if (!commentsDisabled && node.kind !== K.SourceFile) {
			const savedContainerPos = containerPos;
			const savedContainerEnd = containerEnd;
			const savedDeclarationListContainerEnd = declarationListContainerEnd;
			emitCommentsBeforeNode(node);
			pipelineEmitWithSourceMaps(hint, node);
			emitCommentsAfterNode(node, savedContainerPos, savedContainerEnd, savedDeclarationListContainerEnd);
		} else {
			pipelineEmitWithSourceMaps(hint, node);
		}
	}

	function pipelineEmitWithSourceMaps(hint, node) {
		if (!sourceMapsDisabled && node.kind !== K.SourceFile) {
			emitSourceMapsBeforeNode(node);
			emitWithHint(hint, node);
			emitSourceMapsAfterNode(node);
		} else {
			emitWithHint(hint, node);
		}
	}

	// --- source maps --------------------------------------------------------------

	function emitSourceMapsBeforeNode(node) {
		const emitFlags = f.getEmitFlags(node);
		const range = f.getSourceMapRange(node);
		if ((emitFlags & EmitFlags.NoLeadingSourceMap) === 0 && range.pos >= 0) emitPos(skipTrivia(sourceMapSource.text, range.pos));
		if (emitFlags & EmitFlags.NoNestedSourceMaps) sourceMapsDisabled = true;
	}
	function emitSourceMapsAfterNode(node) {
		const emitFlags = f.getEmitFlags(node);
		const range = f.getSourceMapRange(node);
		if (emitFlags & EmitFlags.NoNestedSourceMaps) sourceMapsDisabled = false;
		if ((emitFlags & EmitFlags.NoTrailingSourceMap) === 0 && range.end >= 0) emitPos(range.end);
	}
	function emitPos(pos) {
		if (sourceMapsDisabled || pos < 0) return;
		const { line, character } = computeLineAndCharacterOfPosition(sourceMapSource.lineMap, pos);
		sourceMapGenerator.addMapping(writer.getLine(), writer.getColumn(), sourceMapSourceIndex, line, character, undefined);
	}
	function emitTokenWithSourceMap(node, token, writerFn, tokenPos) {
		if (sourceMapsDisabled) return writeTokenText(token, writerFn, tokenPos);
		const emitFlags = node?.emitNode?.flags ?? 0;
		tokenPos = skipTrivia(sourceMapSource.text, tokenPos);
		if ((emitFlags & EmitFlags.NoTokenLeadingSourceMaps) === 0 && tokenPos >= 0) emitPos(tokenPos);
		tokenPos = writeTokenText(token, writerFn, tokenPos);
		if ((emitFlags & EmitFlags.NoTokenTrailingSourceMaps) === 0 && tokenPos >= 0) emitPos(tokenPos);
		return tokenPos;
	}

	// --- comments ----------------------------------------------------------------

	function emitCommentsBeforeNode(node) {
		const emitFlags = f.getEmitFlags(node);
		const commentRange = f.getCommentRange(node);
		emitLeadingCommentsOfNode(node, emitFlags, commentRange.pos, commentRange.end);
		if (emitFlags & EmitFlags.NoNestedComments) commentsDisabled = true;
	}
	function emitCommentsAfterNode(node, savedContainerPos, savedContainerEnd, savedDeclarationListContainerEnd) {
		const emitFlags = f.getEmitFlags(node);
		const commentRange = f.getCommentRange(node);
		if (emitFlags & EmitFlags.NoNestedComments) commentsDisabled = false;
		emitTrailingCommentsOfNode(node, emitFlags, commentRange.pos, commentRange.end, savedContainerPos, savedContainerEnd, savedDeclarationListContainerEnd);
	}
	function emitLeadingCommentsOfNode(node, emitFlags, pos, end) {
		hasWrittenComment = false;
		const skipLeadingComments = pos < 0 || (emitFlags & EmitFlags.NoLeadingComments) !== 0 || node.kind === K.JsxText;
		const skipTrailingComments = end < 0 || (emitFlags & EmitFlags.NoTrailingComments) !== 0 || node.kind === K.JsxText;
		if ((pos > 0 || end > 0) && pos !== end) {
			if (!skipLeadingComments) emitLeadingComments(pos, node.kind !== K.NotEmittedStatement);
			if (!skipLeadingComments || (pos >= 0 && (emitFlags & EmitFlags.NoLeadingComments) !== 0)) containerPos = pos;
			if (!skipTrailingComments || (end >= 0 && (emitFlags & EmitFlags.NoTrailingComments) !== 0)) {
				containerEnd = end;
				if (node.kind === K.VariableDeclarationList) declarationListContainerEnd = end;
			}
		}
	}
	function emitTrailingCommentsOfNode(node, emitFlags, pos, end, savedContainerPos, savedContainerEnd, savedDeclarationListContainerEnd) {
		const skipTrailingComments = end < 0 || (emitFlags & EmitFlags.NoTrailingComments) !== 0 || node.kind === K.JsxText;
		if ((pos > 0 || end > 0) && pos !== end) {
			containerPos = savedContainerPos;
			containerEnd = savedContainerEnd;
			declarationListContainerEnd = savedDeclarationListContainerEnd;
			if (!skipTrailingComments && node.kind !== K.NotEmittedStatement) emitTrailingComments(end);
		}
	}
	function emitLeadingComments(pos, isEmittedNode) {
		hasWrittenComment = false;
		if (isEmittedNode) {
			if (pos === 0 && currentSourceFile?.isDeclarationFile) forEachLeadingCommentToEmit(pos, emitNonTripleSlashLeadingComment);
			else forEachLeadingCommentToEmit(pos, emitLeadingComment);
		} else if (pos === 0) {
			forEachLeadingCommentToEmit(pos, emitTripleSlashLeadingComment);
		}
	}
	function emitTripleSlashLeadingComment(commentPos, commentEnd, kind, hasTrailingNewLine, rangePos) {
		if (isRecognizedTripleSlashComment(currentSourceFile.text, commentPos, commentEnd)) emitLeadingComment(commentPos, commentEnd, kind, hasTrailingNewLine, rangePos);
	}
	function emitNonTripleSlashLeadingComment(commentPos, commentEnd, kind, hasTrailingNewLine, rangePos) {
		if (!isRecognizedTripleSlashComment(currentSourceFile.text, commentPos, commentEnd)) emitLeadingComment(commentPos, commentEnd, kind, hasTrailingNewLine, rangePos);
	}
	function shouldWriteComment(text, pos) {
		if (printerOptions.onlyPrintJsDocStyle) return isJSDocLikeText(text, pos) || isPinnedComment(text, pos);
		return true;
	}
	function emitLeadingComment(commentPos, commentEnd, kind, hasTrailingNewLine, rangePos) {
		if (!currentSourceFile || !shouldWriteComment(currentSourceFile.text, commentPos)) return;
		if (!hasWrittenComment) {
			emitNewLineBeforeLeadingCommentOfPosition(getCurrentLineMap(), rangePos, commentPos);
			hasWrittenComment = true;
		}
		emitPos(commentPos);
		writeCommentRange(currentSourceFile.text, getCurrentLineMap(), commentPos, commentEnd);
		emitPos(commentEnd);
		if (hasTrailingNewLine) writer.writeLine();
		else if (kind === K.MultiLineCommentTrivia) writer.writeSpace(" ");
	}
	function emitLeadingCommentsOfPosition(pos) {
		if (commentsDisabled || pos === -1) return;
		emitLeadingComments(pos, true);
	}
	function emitTrailingComments(pos) {
		forEachTrailingCommentToEmit(pos, emitTrailingComment);
	}
	function emitTrailingComment(commentPos, commentEnd, _kind, hasTrailingNewLine) {
		if (!currentSourceFile || !shouldWriteComment(currentSourceFile.text, commentPos)) return;
		if (!writer.isAtStartOfLine()) writer.writeSpace(" ");
		emitPos(commentPos);
		writeCommentRange(currentSourceFile.text, getCurrentLineMap(), commentPos, commentEnd);
		emitPos(commentEnd);
		if (hasTrailingNewLine) writer.writeLine();
	}
	function emitTrailingCommentsOfPosition(pos, prefixSpace, forceNoNewline) {
		if (commentsDisabled) return;
		forEachTrailingCommentToEmit(pos, prefixSpace ? emitTrailingComment : forceNoNewline ? emitTrailingCommentOfPositionNoNewline : emitTrailingCommentOfPosition);
	}
	function emitTrailingCommentOfPositionNoNewline(commentPos, commentEnd, kind) {
		if (!currentSourceFile) return;
		emitPos(commentPos);
		writeCommentRange(currentSourceFile.text, getCurrentLineMap(), commentPos, commentEnd);
		emitPos(commentEnd);
		if (kind === K.SingleLineCommentTrivia) writer.writeLine();
	}
	function emitTrailingCommentOfPosition(commentPos, commentEnd, _kind, hasTrailingNewLine) {
		if (!currentSourceFile) return;
		emitPos(commentPos);
		writeCommentRange(currentSourceFile.text, getCurrentLineMap(), commentPos, commentEnd);
		emitPos(commentEnd);
		if (hasTrailingNewLine) writer.writeLine();
		else writer.writeSpace(" ");
	}
	function forEachLeadingCommentToEmit(pos, cb) {
		if (currentSourceFile && (containerPos === -1 || pos !== containerPos)) {
			if (detachedCommentsInfo !== undefined && detachedCommentsInfo[detachedCommentsInfo.length - 1].nodePos === pos) {
				const detachedPos = detachedCommentsInfo[detachedCommentsInfo.length - 1].detachedCommentEndPos;
				if (detachedCommentsInfo.length - 1) detachedCommentsInfo.pop();
				else detachedCommentsInfo = undefined;
				forEachLeadingCommentRange(currentSourceFile.text, detachedPos, cb, detachedPos);
			} else {
				forEachLeadingCommentRange(currentSourceFile.text, pos, cb, pos);
			}
		}
	}
	function forEachTrailingCommentToEmit(end, cb) {
		if (currentSourceFile && (containerEnd === -1 || (end !== containerEnd && end !== declarationListContainerEnd))) {
			forEachTrailingCommentRange(currentSourceFile.text, end, cb);
		}
	}
	function emitNewLineBeforeLeadingCommentOfPosition(lineMap, pos, commentPos) {
		if (pos !== commentPos && computeLineOfPosition(lineMap, pos) !== computeLineOfPosition(lineMap, commentPos)) writer.writeLine();
	}
	function calculateIndent(text, pos, end) {
		let indent = 0;
		for (; pos < end && isWhiteSpaceSingleLine(text.charCodeAt(pos)); pos++) {
			if (text.charCodeAt(pos) === 9) indent += 4 - (indent % 4);
			else indent++;
		}
		return indent;
	}
	function writeCommentRange(text, lineMap, commentPos, commentEnd) {
		if (text.charCodeAt(commentPos + 1) === 42) {
			const first = computeLineAndCharacterOfPosition(lineMap, commentPos);
			const lineCount = lineMap.length;
			let firstCommentLineIndent;
			for (let pos = commentPos, currentLine = first.line; pos < commentEnd; currentLine++) {
				const nextLineStart = currentLine + 1 === lineCount ? text.length + 1 : lineMap[currentLine + 1];
				if (pos !== commentPos) {
					if (firstCommentLineIndent === undefined) firstCommentLineIndent = calculateIndent(text, lineMap[first.line], commentPos);
					const spacesToEmit = writer.getIndent() * 4 - firstCommentLineIndent + calculateIndent(text, pos, nextLineStart);
					if (spacesToEmit > 0) {
						let singleSpaces = spacesToEmit % 4;
						writer.rawWrite("    ".repeat((spacesToEmit - singleSpaces) / 4));
						while (singleSpaces) {
							writer.rawWrite(" ");
							singleSpaces--;
						}
					} else {
						writer.rawWrite("");
					}
				}
				const end = Math.min(commentEnd, nextLineStart - 1);
				const currentLineText = text.substring(pos, end).trim();
				if (currentLineText) {
					writer.writeComment(currentLineText);
					if (end !== commentEnd) writer.writeLine();
				} else {
					writer.rawWrite(newLine);
				}
				pos = nextLineStart;
			}
		} else {
			writer.writeComment(text.substring(commentPos, commentEnd));
		}
	}
	function emitBodyWithDetachedComments(node, detachedRange, emitCallback) {
		const { pos, end } = detachedRange;
		const emitFlags = f.getEmitFlags(node);
		const skipLeadingComments = pos < 0 || (emitFlags & EmitFlags.NoLeadingComments) !== 0;
		const skipTrailingComments = commentsDisabled || end < 0 || (emitFlags & EmitFlags.NoTrailingComments) !== 0;
		if (!skipLeadingComments) emitDetachedCommentsAndUpdateCommentsInfo(detachedRange);
		if (emitFlags & EmitFlags.NoNestedComments && !commentsDisabled) {
			commentsDisabled = true;
			emitCallback(node);
			commentsDisabled = false;
		} else {
			emitCallback(node);
		}
		if (!skipTrailingComments) {
			emitLeadingComments(detachedRange.end, true);
			if (hasWrittenComment && !writer.isAtStartOfLine()) writer.writeLine();
		}
	}
	function emitDetachedCommentsAndUpdateCommentsInfo(range) {
		const info = currentSourceFile && emitDetachedComments(currentSourceFile.text, getCurrentLineMap(), range);
		if (info) {
			if (detachedCommentsInfo) detachedCommentsInfo.push(info);
			else detachedCommentsInfo = [info];
		}
	}
	function emitDetachedComments(text, lineMap, node) {
		let leadingComments;
		if (commentsDisabled) {
			if (node.pos === 0) leadingComments = (getLeadingCommentRanges(text, node.pos) ?? []).filter((comment) => isPinnedComment(text, comment.pos));
		} else {
			leadingComments = getLeadingCommentRanges(text, node.pos);
		}
		if (!leadingComments || !leadingComments.length) return undefined;
		const detachedComments = [];
		let lastComment;
		for (const comment of leadingComments) {
			if (lastComment) {
				const lastCommentLine = computeLineOfPosition(lineMap, lastComment.end);
				const commentLine = computeLineOfPosition(lineMap, comment.pos);
				if (commentLine >= lastCommentLine + 2) break;
			}
			detachedComments.push(comment);
			lastComment = comment;
		}
		if (!detachedComments.length) return undefined;
		const lastCommentLine = computeLineOfPosition(lineMap, detachedComments[detachedComments.length - 1].end);
		const nodeLine = computeLineOfPosition(lineMap, skipTrivia(text, node.pos));
		if (nodeLine < lastCommentLine + 2) return undefined;
		// emitNewLineBeforeLeadingComments, then emitComments with a trailing separator
		if (leadingComments.length && node.pos !== leadingComments[0].pos && computeLineOfPosition(lineMap, node.pos) !== computeLineOfPosition(lineMap, leadingComments[0].pos)) {
			writer.writeLine();
		}
		let emitInterveningSeparator = false;
		for (const comment of detachedComments) {
			if (emitInterveningSeparator) {
				writer.writeSpace(" ");
				emitInterveningSeparator = false;
			}
			if (shouldWriteComment(text, comment.pos)) {
				emitPos(comment.pos);
				writeCommentRange(text, lineMap, comment.pos, comment.end);
				emitPos(comment.end);
			}
			if (comment.hasTrailingNewLine) writer.writeLine();
			else emitInterveningSeparator = true;
		}
		if (emitInterveningSeparator) writer.writeSpace(" ");
		return { nodePos: node.pos, detachedCommentEndPos: detachedComments[detachedComments.length - 1].end };
	}

	// --- writing --------------------------------------------------------------------

	function writeLine(count = 1) {
		for (let i = 0; i < count; i++) writer.writeLine(i > 0);
	}

	function writeTokenText(token, writerFn, pos) {
		const tokenString = tokenToString(token);
		writerFn(tokenString);
		return pos < 0 ? pos : pos + tokenString.length;
	}

	function emitTokenWithComment(token, pos, writerFn, contextNode, indentLeading) {
		const node = f.getParseTreeNode(contextNode);
		const isSimilarNode = node && node.kind === contextNode.kind;
		const startPos = pos;
		if (isSimilarNode && currentSourceFile) pos = skipTrivia(currentSourceFile.text, pos);
		if (isSimilarNode && contextNode.pos !== startPos) {
			const needsIndent = indentLeading && currentSourceFile && computeLineOfPosition(getCurrentLineMap(), startPos) !== computeLineOfPosition(getCurrentLineMap(), pos);
			if (needsIndent) increaseIndent();
			emitLeadingCommentsOfPosition(startPos);
			if (needsIndent) decreaseIndent();
		}
		if (!omitBraceSourcePositions && (token === K.OpenBraceToken || token === K.CloseBraceToken)) {
			pos = sourceMapsDisabled ? writeTokenText(token, writerFn, pos) : emitTokenWithSourceMap(contextNode, token, writerFn, pos);
		} else {
			pos = writeTokenText(token, writerFn, pos);
		}
		if (isSimilarNode && contextNode.end !== pos) emitTrailingCommentsOfPosition(pos, true, false);
		return pos;
	}

	function emitNodeWithWriter(node, writerFn) {
		if (!node) return;
		const savedWrite = write;
		write = writerFn;
		emit(node);
		write = savedWrite;
	}

	// --- lists ------------------------------------------------------------------------

	function getOpeningBracket(format) {
		if (format & ListFormat.Braces) return "{";
		if (format & ListFormat.Parenthesis) return "(";
		if (format & ListFormat.AngleBrackets) return "<";
		return "[";
	}
	function getClosingBracket(format) {
		if (format & ListFormat.Braces) return "}";
		if (format & ListFormat.Parenthesis) return ")";
		if (format & ListFormat.AngleBrackets) return ">";
		return "]";
	}

	function emitList(parentNode, children, format, parenthesizerRule, start, count) {
		emitNodeList(emit, parentNode, children, format | (parentNode && f.getEmitFlags(parentNode) & EmitFlags.MultiLine ? ListFormat.PreferNewLine : 0), parenthesizerRule, start, count);
	}

	function emitNodeList(emitFn, parentNode, children, format, parenthesizerRule, start = 0, count = children ? children.length - start : 0) {
		const isUndefined = children === undefined;
		if (isUndefined && format & ListFormat.OptionalIfUndefined) return;
		const isEmpty = children === undefined || start >= children.length || count === 0;
		if (isEmpty && format & ListFormat.OptionalIfEmpty) return;
		if (format & ListFormat.BracketsMask) {
			writePunctuation(getOpeningBracket(format));
			if (isEmpty && children) emitTrailingCommentsOfPosition(children.pos, true);
		}
		if (isEmpty) {
			if (format & ListFormat.MultiLine) writeLine();
			else if (format & ListFormat.SpaceBetweenBraces && !(format & ListFormat.NoSpaceIfEmpty)) writeSpace();
		} else {
			emitNodeListItems(emitFn, parentNode, children, format, parenthesizerRule, start, count, children.hasTrailingComma, children);
		}
		if (format & ListFormat.BracketsMask) {
			if (isEmpty && children) emitLeadingCommentsOfPosition(children.end);
			writePunctuation(getClosingBracket(format));
		}
	}

	function emitNodeListItems(emitFn, parentNode, children, format, parenthesizerRule, start, count, hasTrailingComma, childrenTextRange) {
		const mayEmitInterveningComments = (format & ListFormat.NoInterveningComments) === 0;
		let shouldEmitInterveningComments = mayEmitInterveningComments;
		const leadingLineTerminatorCount = getLeadingLineTerminatorCount(parentNode, children[start], format);
		if (leadingLineTerminatorCount) {
			writeLine(leadingLineTerminatorCount);
			shouldEmitInterveningComments = false;
		} else if (format & ListFormat.SpaceBetweenBraces) {
			writeSpace();
		}
		if (format & ListFormat.Indented) increaseIndent();
		let previousSibling;
		let shouldDecreaseIndentAfterEmit = false;
		for (let i = 0; i < count; i++) {
			const child = children[start + i];
			if (format & ListFormat.AsteriskDelimited) {
				writeLine();
				writeDelimiter(format);
			} else if (previousSibling) {
				if (format & ListFormat.DelimitersMask && previousSibling.end !== (parentNode ? parentNode.end : -1)) {
					if (!(f.getEmitFlags(previousSibling) & EmitFlags.NoTrailingComments)) emitLeadingCommentsOfPosition(previousSibling.end);
				}
				writeDelimiter(format);
				const separatingLineTerminatorCount = getSeparatingLineTerminatorCount(previousSibling, child, format);
				if (separatingLineTerminatorCount > 0) {
					if ((format & (ListFormat.LinesMask | ListFormat.Indented)) === ListFormat.SingleLine) {
						increaseIndent();
						shouldDecreaseIndentAfterEmit = true;
					}
					if (shouldEmitInterveningComments && format & ListFormat.DelimitersMask && child.pos >= 0) {
						emitTrailingCommentsOfPosition(f.getCommentRange(child).pos, !!(format & ListFormat.SpaceBetweenSiblings), true);
					}
					writeLine(separatingLineTerminatorCount);
					shouldEmitInterveningComments = false;
				} else if (previousSibling && format & ListFormat.SpaceBetweenSiblings) {
					writeSpace();
				}
			}
			if (shouldEmitInterveningComments) emitTrailingCommentsOfPosition(f.getCommentRange(child).pos);
			else shouldEmitInterveningComments = mayEmitInterveningComments;
			nextListElementPos = child.pos;
			emitFn(child);
			if (shouldDecreaseIndentAfterEmit) {
				decreaseIndent();
				shouldDecreaseIndentAfterEmit = false;
			}
			previousSibling = child;
		}
		const emitFlags = previousSibling ? f.getEmitFlags(previousSibling) : 0;
		const skipTrailingComments = commentsDisabled || !!(emitFlags & EmitFlags.NoTrailingComments);
		const emitTrailingComma = hasTrailingComma && format & ListFormat.AllowTrailingComma && format & ListFormat.CommaDelimited;
		if (emitTrailingComma) {
			if (previousSibling && !skipTrailingComments) emitTokenWithComment(K.CommaToken, previousSibling.end, writePunctuation, previousSibling);
			else writePunctuation(",");
		}
		if (previousSibling && (parentNode ? parentNode.end : -1) !== previousSibling.end && format & ListFormat.DelimitersMask && !skipTrailingComments) {
			emitLeadingCommentsOfPosition(emitTrailingComma && childrenTextRange?.end ? childrenTextRange.end : previousSibling.end);
		}
		if (format & ListFormat.Indented) decreaseIndent();
		const closingLineTerminatorCount = getClosingLineTerminatorCount(parentNode, children[start + count - 1], format, childrenTextRange);
		if (closingLineTerminatorCount) writeLine(closingLineTerminatorCount);
		else if (format & (ListFormat.SpaceAfterList | ListFormat.SpaceBetweenBraces)) writeSpace();
	}

	function writeDelimiter(format) {
		switch (format & ListFormat.DelimitersMask) {
			case ListFormat.CommaDelimited:
				writePunctuation(",");
				break;
			case ListFormat.BarDelimited:
				writeSpace();
				writePunctuation("|");
				break;
			case ListFormat.AsteriskDelimited:
				writeSpace();
				writePunctuation("*");
				writeSpace();
				break;
			case ListFormat.AmpersandDelimited:
				writeSpace();
				writePunctuation("&");
				break;
		}
	}

	function lineOf(pos) {
		return computeLineOfPosition(getCurrentLineMap(), pos);
	}
	function startOfRange(range) {
		return range.pos < 0 ? -1 : skipTrivia(currentSourceFile.text, range.pos);
	}
	function rangeIsOnSingleLine(range) {
		return lineOf(startOfRange(range)) === lineOf(range.end);
	}
	function synthesizedNodeStartsOnNewLine(node, format) {
		if (f.nodeIsSynthesized(node)) {
			const startsOnNewLine = f.getStartsOnNewLine(node);
			if (startsOnNewLine === undefined) return (format & ListFormat.PreferNewLine) !== 0;
			return startsOnNewLine;
		}
		return (format & ListFormat.PreferNewLine) !== 0;
	}
	function getLeadingLineTerminatorCount(parentNode, firstChild, format) {
		if (format & ListFormat.PreserveLines) {
			if (format & ListFormat.PreferNewLine) return 1;
			if (firstChild === undefined) return !parentNode || (currentSourceFile && rangeIsOnSingleLine(parentNode)) ? 0 : 1;
			if (firstChild.pos === nextListElementPos) return 0;
			if (firstChild.kind === K.JsxText) return 0;
			if (
				currentSourceFile &&
				parentNode &&
				parentNode.pos >= 0 &&
				!f.nodeIsSynthesized(firstChild) &&
				(!firstChild.parent || f.getOriginalNode(firstChild.parent) === f.getOriginalNode(parentNode))
			) {
				return lineOf(startOfRange(parentNode)) === lineOf(startOfRange(firstChild)) ? 0 : 1;
			}
			if (synthesizedNodeStartsOnNewLine(firstChild, format)) return 1;
		}
		return format & ListFormat.MultiLine ? 1 : 0;
	}
	function getSeparatingLineTerminatorCount(previousNode, nextNode, format) {
		if (format & ListFormat.PreserveLines) {
			if (previousNode === undefined || nextNode === undefined) return 0;
			if (nextNode.kind === K.JsxText) return 0;
			if (currentSourceFile && !f.nodeIsSynthesized(previousNode) && !f.nodeIsSynthesized(nextNode)) {
				const previousOriginal = f.getOriginalNode(previousNode);
				if (previousOriginal.parent && previousOriginal.parent === f.getOriginalNode(nextNode).parent) {
					return lineOf(previousNode.end) === lineOf(startOfRange(nextNode)) ? 0 : 1;
				}
				return format & ListFormat.PreferNewLine ? 1 : 0;
			}
			if (synthesizedNodeStartsOnNewLine(previousNode, format) || synthesizedNodeStartsOnNewLine(nextNode, format)) return 1;
		} else if (f.getStartsOnNewLine(nextNode)) {
			return 1;
		}
		return format & ListFormat.MultiLine ? 1 : 0;
	}
	function getClosingLineTerminatorCount(parentNode, lastChild, format) {
		if (format & ListFormat.PreserveLines) {
			if (format & ListFormat.PreferNewLine) return 1;
			if (lastChild === undefined) return !parentNode || (currentSourceFile && rangeIsOnSingleLine(parentNode)) ? 0 : 1;
			if (currentSourceFile && parentNode && parentNode.pos >= 0 && !f.nodeIsSynthesized(lastChild) && (!lastChild.parent || lastChild.parent === parentNode)) {
				return lineOf(parentNode.end) === lineOf(lastChild.end) ? 0 : 1;
			}
			if (synthesizedNodeStartsOnNewLine(lastChild, format)) return 1;
		}
		if (format & ListFormat.MultiLine && !(format & ListFormat.NoTrailingNewLine)) return 1;
		return 0;
	}

	// --- nodes ---------------------------------------------------------------------------

	function emitWithHint(hint, node) {
		if (hint === 0) return emitSourceFile(node);
		if (hint === 2) return emitIdentifier(node);
		if (hint === 3) return emitMappedTypeParameter(node);
		switch (node.kind) {
			case K.TemplateHead:
			case K.TemplateMiddle:
			case K.TemplateTail:
				return emitLiteral(node);
			case K.Identifier:
				return emitIdentifier(node);
			case K.PrivateIdentifier:
				return write(getTextOfNode(node));
			case K.QualifiedName:
				return emitQualifiedName(node);
			case K.ComputedPropertyName:
				writePunctuation("[");
				emitExpression(node.expression);
				return writePunctuation("]");
			case K.TypeParameter:
				return emitTypeParameter(node);
			case K.Parameter:
				return emitParameter(node);
			case K.PropertySignature:
				return emitPropertySignature(node);
			case K.PropertyDeclaration:
				return emitPropertyDeclaration(node);
			case K.MethodSignature:
				emitModifierList(node, node.modifiers);
				emit(node.name);
				emit(node.questionToken);
				return emitSignatureHead(node, true);
			case K.MethodDeclaration:
				emitModifiers(node, node.modifiers);
				emit(node.asteriskToken);
				emit(node.name);
				emit(node.questionToken);
				return emitSignatureHead(node, true);
			case K.Constructor:
				emitModifiers(node, node.modifiers);
				writeKeyword("constructor");
				return emitSignatureHead(node, true);
			case K.GetAccessor:
			case K.SetAccessor:
				return emitAccessorDeclaration(node);
			case K.CallSignature:
				return emitSignatureHead(node, true);
			case K.ConstructSignature:
				writeKeyword("new");
				writeSpace();
				return emitSignatureHead(node, true);
			case K.IndexSignature:
				emitModifiers(node, node.modifiers);
				emitList(node, node.parameters, ListFormat.IndexSignatureParameters);
				emitTypeAnnotation(node.type);
				return writeTrailingSemicolon();
			case K.TypePredicate:
				if (node.assertsModifier) {
					emit(node.assertsModifier);
					writeSpace();
				}
				emit(node.parameterName);
				if (node.type) {
					writeSpace();
					writeKeyword("is");
					writeSpace();
					emit(node.type);
				}
				return;
			case K.TypeReference:
				emit(node.typeName);
				return emitTypeArguments(node, node.typeArguments);
			case K.FunctionType:
				return emitFunctionType(node);
			case K.ConstructorType:
				emitModifierList(node, node.modifiers);
				writeKeyword("new");
				writeSpace();
				return emitFunctionType(node);
			case K.TypeQuery:
				writeKeyword("typeof");
				writeSpace();
				emit(node.exprName);
				return emitTypeArguments(node, node.typeArguments);
			case K.TypeLiteral:
				writePunctuation("{");
				emitList(node, node.members, (f.getEmitFlags(node) & EmitFlags.SingleLine ? ListFormat.SingleLineTypeLiteralMembers : ListFormat.MultiLineTypeLiteralMembers) | ListFormat.NoSpaceIfEmpty);
				return writePunctuation("}");
			case K.ArrayType:
				emit(node.elementType, parenthesizer.parenthesizeNonArrayTypeOfPostfixType);
				writePunctuation("[");
				return writePunctuation("]");
			case K.TupleType:
				emitTokenWithComment(K.OpenBracketToken, node.pos, writePunctuation, node);
				emitList(
					node,
					node.elements,
					(f.getEmitFlags(node) & EmitFlags.SingleLine ? ListFormat.SingleLineTupleTypeElements : ListFormat.MultiLineTupleTypeElements) | ListFormat.NoSpaceIfEmpty,
					parenthesizer.parenthesizeElementTypeOfTupleType,
				);
				return emitTokenWithComment(K.CloseBracketToken, node.elements.end, writePunctuation, node);
			case K.OptionalType:
				emit(node.type, parenthesizer.parenthesizeTypeOfOptionalType);
				return writePunctuation("?");
			case K.RestType:
				writePunctuation("...");
				return emit(node.type);
			case K.UnionType:
				return emitList(node, node.types, ListFormat.UnionTypeConstituents, parenthesizer.parenthesizeConstituentTypeOfUnionType);
			case K.IntersectionType:
				return emitList(node, node.types, ListFormat.IntersectionTypeConstituents, parenthesizer.parenthesizeConstituentTypeOfIntersectionType);
			case K.ConditionalType:
				emit(node.checkType, parenthesizer.parenthesizeCheckTypeOfConditionalType);
				writeSpace();
				writeKeyword("extends");
				writeSpace();
				emit(node.extendsType, parenthesizer.parenthesizeExtendsTypeOfConditionalType);
				writeSpace();
				writePunctuation("?");
				writeSpace();
				emit(node.trueType);
				writeSpace();
				writePunctuation(":");
				writeSpace();
				return emit(node.falseType);
			case K.InferType:
				writeKeyword("infer");
				writeSpace();
				return emit(node.typeParameter);
			case K.ParenthesizedType:
				writePunctuation("(");
				emit(node.type);
				return writePunctuation(")");
			case K.ExpressionWithTypeArguments:
				emitExpression(node.expression);
				return emitTypeArguments(node, node.typeArguments);
			case K.ThisType:
				return writeKeyword("this");
			case K.TypeOperator:
				writeTokenText(node.operator, writeKeyword);
				writeSpace();
				return emit(node.type, node.operator === K.ReadonlyKeyword ? parenthesizer.parenthesizeOperandOfReadonlyTypeOperator : parenthesizer.parenthesizeOperandOfTypeOperator);
			case K.IndexedAccessType:
				emit(node.objectType, parenthesizer.parenthesizeNonArrayTypeOfPostfixType);
				writePunctuation("[");
				emit(node.indexType);
				return writePunctuation("]");
			case K.MappedType:
				return emitMappedType(node);
			case K.LiteralType:
				return emitExpression(node.literal);
			case K.NamedTupleMember:
				emit(node.dotDotDotToken);
				emit(node.name);
				emit(node.questionToken);
				emitTokenWithComment(K.ColonToken, node.name.end, writePunctuation, node);
				writeSpace();
				return emit(node.type);
			case K.TemplateLiteralType:
				emit(node.head);
				return emitList(node, node.templateSpans, ListFormat.TemplateExpressionSpans);
			case K.TemplateLiteralTypeSpan:
				emit(node.type);
				return emit(node.literal);
			case K.ImportType:
				return emitImportTypeNode(node);
			case K.ObjectBindingPattern:
				writePunctuation("{");
				emitList(node, node.elements, ListFormat.ObjectBindingPatternElements);
				return writePunctuation("}");
			case K.ArrayBindingPattern:
				writePunctuation("[");
				emitList(node, node.elements, ListFormat.ArrayBindingPatternElements);
				return writePunctuation("]");
			case K.BindingElement:
				emit(node.dotDotDotToken);
				if (node.propertyName) {
					emit(node.propertyName);
					writePunctuation(":");
					writeSpace();
				}
				emit(node.name);
				return emitInitializer(node.initializer, node.name.end, node);
			case K.VariableStatement:
				emitModifiers(node, node.modifiers);
				emit(node.declarationList);
				return writeTrailingSemicolon();
			case K.VariableDeclaration:
				emit(node.name);
				emit(node.exclamationToken);
				emitTypeAnnotation(node.type);
				return emitInitializer(node.initializer, node.type?.end ?? node.name.end, node);
			case K.VariableDeclarationList:
				writeKeyword(node.flags & NodeFlags.Let ? "let" : node.flags & NodeFlags.Const ? "const" : node.flags & NodeFlags.Using ? "using" : "var");
				writeSpace();
				return emitList(node, node.declarations, ListFormat.VariableDeclarationList);
			case K.FunctionDeclaration:
				emitModifiers(node, node.modifiers);
				writeKeyword("function");
				emit(node.asteriskToken);
				writeSpace();
				emitIdentifierName(node.name);
				return emitSignatureHead(node, true);
			case K.ClassDeclaration:
				return emitClassDeclaration(node);
			case K.InterfaceDeclaration:
				emitModifiers(node, node.modifiers);
				writeKeyword("interface");
				writeSpace();
				emit(node.name);
				emitTypeParameters(node, node.typeParameters);
				emitList(node, node.heritageClauses, ListFormat.HeritageClauses);
				writeSpace();
				writePunctuation("{");
				emitList(node, node.members, ListFormat.InterfaceMembers);
				return writePunctuation("}");
			case K.TypeAliasDeclaration:
				emitModifiers(node, node.modifiers);
				writeKeyword("type");
				writeSpace();
				emit(node.name);
				emitTypeParameters(node, node.typeParameters);
				writeSpace();
				writePunctuation("=");
				writeSpace();
				emit(node.type);
				return writeTrailingSemicolon();
			case K.EnumDeclaration:
				emitModifiers(node, node.modifiers);
				writeKeyword("enum");
				writeSpace();
				emit(node.name);
				writeSpace();
				writePunctuation("{");
				emitList(node, node.members, ListFormat.EnumMembers);
				return writePunctuation("}");
			case K.ModuleDeclaration:
				return emitModuleDeclaration(node);
			case K.ModuleBlock:
				return emitBlockStatements(node, isEmptyBlock(node));
			case K.ImportEqualsDeclaration:
				return emitImportEqualsDeclaration(node);
			case K.ImportDeclaration:
				return emitImportDeclaration(node);
			case K.ImportClause:
				return emitImportClause(node);
			case K.NamespaceImport:
			case K.NamespaceExport: {
				const asPos = emitTokenWithComment(K.AsteriskToken, node.pos, writePunctuation, node);
				writeSpace();
				emitTokenWithComment(K.AsKeyword, asPos, writeKeyword, node);
				writeSpace();
				return emit(node.name);
			}
			case K.NamedImports:
			case K.NamedExports:
				writePunctuation("{");
				emitList(node, node.elements, ListFormat.NamedImportsOrExportsElements);
				return writePunctuation("}");
			case K.ImportSpecifier:
			case K.ExportSpecifier:
				if (node.isTypeOnly) {
					writeKeyword("type");
					writeSpace();
				}
				if (node.propertyName) {
					emit(node.propertyName);
					writeSpace();
					emitTokenWithComment(K.AsKeyword, node.propertyName.end, writeKeyword, node);
					writeSpace();
				}
				return emit(node.name);
			case K.ExportAssignment:
				return emitExportAssignment(node);
			case K.ExportDeclaration:
				return emitExportDeclaration(node);
			case K.ImportAttributes:
				emitTokenWithComment(node.token, node.pos, writeKeyword, node);
				writeSpace();
				return emitList(node, node.elements, ListFormat.ImportAttributes);
			case K.ImportAttribute:
				emit(node.name);
				writePunctuation(":");
				writeSpace();
				if ((f.getEmitFlags(node.value) & EmitFlags.NoLeadingComments) === 0) emitTrailingCommentsOfPosition(f.getCommentRange(node.value).pos);
				return emit(node.value);
			case K.ExternalModuleReference:
				writeKeyword("require");
				writePunctuation("(");
				emitExpression(node.expression);
				return writePunctuation(")");
			case K.HeritageClause:
				writeSpace();
				writeTokenText(node.token, writeKeyword);
				writeSpace();
				return emitList(node, node.types, ListFormat.HeritageClauseTypes);
			case K.EnumMember:
				emit(node.name);
				return emitInitializer(node.initializer, node.name.end, node);
			case K.NamespaceExportDeclaration: {
				let nextPos = emitTokenWithComment(K.ExportKeyword, node.pos, writeKeyword, node);
				writeSpace();
				nextPos = emitTokenWithComment(K.AsKeyword, nextPos, writeKeyword, node);
				writeSpace();
				emitTokenWithComment(K.NamespaceKeyword, nextPos, writeKeyword, node);
				writeSpace();
				emit(node.name);
				return writeTrailingSemicolon();
			}
			case K.NumericLiteral:
			case K.BigIntLiteral:
			case K.StringLiteral:
			case K.NoSubstitutionTemplateLiteral:
				return emitLiteral(node);
			case K.PrefixUnaryExpression:
				writeTokenText(node.operator, writeOperator);
				if (shouldEmitWhitespaceBeforeOperand(node)) writeSpace();
				return emitExpression(node.operand);
			case K.PropertyAccessExpression:
				emitExpression(node.expression);
				emitTokenWithComment(K.DotToken, node.expression.end, writePunctuation, node);
				return emit(node.name);
			case K.ParenthesizedExpression: {
				const openParenPos = emitTokenWithComment(K.OpenParenToken, node.pos, writePunctuation, node);
				emitExpression(node.expression);
				return emitTokenWithComment(K.CloseParenToken, node.expression ? node.expression.end : openParenPos, writePunctuation, node);
			}
			case K.SemicolonClassElement:
				return writeTrailingSemicolon();
		}
		if (isKeyword(node.kind)) return writer.writeKeyword(tokenToString(node.kind));
		if (isTokenKind(node.kind)) return writer.writePunctuation(tokenToString(node.kind));
		throw new Error(`surge-ts: the declaration printer cannot print a node of kind ${node.kind}`);
	}

	function emitSourceFile(node) {
		writeLine();
		const statements = node.statements;
		emitBodyWithDetachedComments(node, statements, emitSourceFileWorker);
	}
	function emitSourceFileWorker(node) {
		if (node.isDeclarationFile) emitTripleSlashDirectives(node.hasNoDefaultLib, node.referencedFiles, node.typeReferenceDirectives, node.libReferenceDirectives);
		emitList(node, node.statements, ListFormat.MultiLine);
	}
	function emitTripleSlashDirectives(hasNoDefaultLib, files, types, libs) {
		if (hasNoDefaultLib) {
			writer.writeComment(`/// <reference no-default-lib="true"/>`);
			writeLine();
		}
		const writeDirectives = (kind, directives) => {
			for (const directive of directives ?? []) {
				const resolutionMode = directive.resolutionMode ? `resolution-mode="${directive.resolutionMode === 99 ? "import" : "require"}" ` : "";
				const preserve = directive.preserve ? `preserve="true" ` : "";
				writer.writeComment(`/// <reference ${kind}="${directive.fileName}" ${resolutionMode}${preserve}/>`);
				writeLine();
			}
		};
		writeDirectives("path", files);
		writeDirectives("types", types);
		writeDirectives("lib", libs);
	}

	function getTextOfNode(node) {
		const sourceFile = currentSourceFile;
		const canUseSourceFile = !!sourceFile && !!node.parent && !f.nodeIsSynthesized(node);
		if (node.kind === K.Identifier || node.kind === K.PrivateIdentifier) {
			if (!canUseSourceFile || getSourceFileOfNode(node) !== f.getOriginalNode(sourceFile)) return idText(node);
		} else if (!canUseSourceFile) {
			return node.text;
		}
		return sourceFile.text.substring(skipTrivia(sourceFile.text, node.pos), node.end);
	}

	function emitIdentifier(node) {
		write(getTextOfNode(node));
	}

	function emitQualifiedName(node) {
		emit(node.left);
		writePunctuation(".");
		emit(node.right);
	}

	function emitLiteral(node) {
		let text;
		if (currentSourceFile && !f.nodeIsSynthesized(node) && node.parent && node.kind !== K.BigIntLiteral) {
			text = currentSourceFile.text.substring(skipTrivia(currentSourceFile.text, node.pos), node.end);
		} else {
			switch (node.kind) {
				case K.StringLiteral: {
					const escapeText = f.getEmitFlags(node) & EmitFlags.NoAsciiEscaping ? escapeString : escapeNonAsciiString;
					text = node.singleQuote ? `'${escapeText(node.text, 39)}'` : `"${escapeText(node.text, 34)}"`;
					break;
				}
				case K.NoSubstitutionTemplateLiteral:
				case K.TemplateHead:
				case K.TemplateMiddle:
				case K.TemplateTail: {
					const escapeText = f.getEmitFlags(node) & EmitFlags.NoAsciiEscaping ? escapeString : escapeNonAsciiString;
					const rawText = node.rawText ?? escapeText(node.text, 96).replace(/\$\{/g, "\\${");
					text =
						node.kind === K.NoSubstitutionTemplateLiteral
							? `\`${rawText}\``
							: node.kind === K.TemplateHead
								? `\`${rawText}\${`
								: node.kind === K.TemplateMiddle
									? `}${rawText}\${`
									: `}${rawText}\``;
					break;
				}
				default:
					text = node.text;
			}
		}
		if (printerOptions.sourceMap && (node.kind === K.StringLiteral || (node.kind >= K.NoSubstitutionTemplateLiteral && node.kind <= K.TemplateTail))) writer.writeLiteral(text);
		else writer.writeStringLiteral(text);
	}

	function shouldEmitWhitespaceBeforeOperand(node) {
		const operand = node.operand;
		return (
			operand.kind === K.PrefixUnaryExpression &&
			((node.operator === K.PlusToken && (operand.operator === K.PlusToken || operand.operator === K.PlusPlusToken)) ||
				(node.operator === K.MinusToken && (operand.operator === K.MinusToken || operand.operator === K.MinusMinusToken)))
		);
	}

	function emitModifiers(node, modifiers) {
		if (modifiers && modifiers.length) emitModifierList(node, modifiers);
	}
	function emitModifierList(node, modifiers) {
		emitList(node, modifiers, ListFormat.Modifiers);
	}
	function emitTypeAnnotation(node) {
		if (node) {
			writePunctuation(":");
			writeSpace();
			emit(node);
		}
	}
	function emitInitializer(node, equalCommentStartPos, container) {
		if (node) {
			writeSpace();
			emitTokenWithComment(K.EqualsToken, equalCommentStartPos, writeOperator, container);
			writeSpace();
			emitExpression(node);
		}
	}
	function emitTypeParameters(parentNode, typeParameters) {
		emitList(parentNode, typeParameters, ListFormat.TypeParameters);
	}
	function emitTypeArguments(parentNode, typeArguments) {
		emitList(parentNode, typeArguments, ListFormat.TypeArguments, { select: (index) => (index === 0 ? parenthesizer.parenthesizeLeadingTypeArgument : undefined) });
	}
	function emitSignatureHead(node, withSemicolon) {
		const indented = f.getEmitFlags(node) & EmitFlags.Indented;
		if (indented) increaseIndent();
		emitTypeParameters(node, node.typeParameters);
		emitList(node, node.parameters, ListFormat.Parameters);
		emitTypeAnnotation(node.type);
		if (withSemicolon) writeTrailingSemicolon();
		if (indented) decreaseIndent();
	}
	function emitFunctionType(node) {
		emitTypeParameters(node, node.typeParameters);
		emitList(node, node.parameters, ListFormat.Parameters);
		writeSpace();
		writePunctuation("=>");
		writeSpace();
		emit(node.type);
	}
	function emitTypeParameter(node) {
		emitModifierList(node, node.modifiers);
		emit(node.name);
		if (node.constraint) {
			writeSpace();
			writeKeyword("extends");
			writeSpace();
			emit(node.constraint);
		}
		if (node.default) {
			writeSpace();
			writeOperator("=");
			writeSpace();
			emit(node.default);
		}
	}
	function emitParameter(node) {
		emitModifiers(node, node.modifiers);
		emit(node.dotDotDotToken);
		emitNodeWithWriter(node.name, (s) => writer.writeParameter(s));
		emit(node.questionToken);
		emitTypeAnnotation(node.type);
		emitInitializer(
			node.initializer,
			node.type ? node.type.end : node.questionToken ? node.questionToken.end : node.name ? node.name.end : node.modifiers ? node.modifiers.end : node.pos,
			node,
		);
	}
	function emitPropertySignature(node) {
		emitModifierList(node, node.modifiers);
		emitNodeWithWriter(node.name, (s) => writer.writeProperty(s));
		emit(node.questionToken);
		emitTypeAnnotation(node.type);
		writeTrailingSemicolon();
	}
	function emitPropertyDeclaration(node) {
		emitModifiers(node, node.modifiers);
		emit(node.name);
		emit(node.questionToken);
		emit(node.exclamationToken);
		emitTypeAnnotation(node.type);
		emitInitializer(node.initializer, node.type ? node.type.end : node.questionToken ? node.questionToken.end : node.name.end, node);
		writeTrailingSemicolon();
	}
	function emitAccessorDeclaration(node) {
		emitModifiers(node, node.modifiers);
		const lastModifier = node.modifiers && node.modifiers.length ? node.modifiers[node.modifiers.length - 1] : undefined;
		const pos = lastModifier && lastModifier.end >= 0 ? lastModifier.end : node.pos;
		emitTokenWithComment(node.kind === K.GetAccessor ? K.GetKeyword : K.SetKeyword, pos, writeKeyword, node);
		writeSpace();
		emit(node.name);
		emitSignatureHead(node, true);
	}
	function emitMappedType(node) {
		const emitFlags = f.getEmitFlags(node);
		writePunctuation("{");
		if (emitFlags & EmitFlags.SingleLine) writeSpace();
		else {
			writeLine();
			increaseIndent();
		}
		if (node.readonlyToken) {
			emit(node.readonlyToken);
			if (node.readonlyToken.kind !== K.ReadonlyKeyword) writeKeyword("readonly");
			writeSpace();
		}
		writePunctuation("[");
		pipelineEmit(3, node.typeParameter);
		if (node.nameType) {
			writeSpace();
			writeKeyword("as");
			writeSpace();
			emit(node.nameType);
		}
		writePunctuation("]");
		if (node.questionToken) {
			emit(node.questionToken);
			if (node.questionToken.kind !== K.QuestionToken) writePunctuation("?");
		}
		writePunctuation(":");
		writeSpace();
		emit(node.type);
		writeTrailingSemicolon();
		if (emitFlags & EmitFlags.SingleLine) writeSpace();
		else {
			writeLine();
			decreaseIndent();
		}
		emitList(node, node.members, ListFormat.PreserveLines);
		writePunctuation("}");
	}
	function emitMappedTypeParameter(node) {
		emit(node.name);
		writeSpace();
		writeKeyword("in");
		writeSpace();
		emit(node.constraint);
	}
	function emitImportTypeNode(node) {
		if (node.isTypeOf) {
			writeKeyword("typeof");
			writeSpace();
		}
		writeKeyword("import");
		writePunctuation("(");
		emit(node.argument);
		if (node.attributes) {
			writePunctuation(",");
			writeSpace();
			writePunctuation("{");
			writeSpace();
			writeKeyword(node.attributes.token === K.AssertKeyword ? "assert" : "with");
			writePunctuation(":");
			writeSpace();
			const elements = node.attributes.elements;
			emitList(node.attributes, elements, ListFormat.ImportAttributes);
			writeSpace();
			writePunctuation("}");
		}
		writePunctuation(")");
		if (node.qualifier) {
			writePunctuation(".");
			emit(node.qualifier);
		}
		emitTypeArguments(node, node.typeArguments);
	}
	function moveRangePastModifiersPos(node) {
		if (node.kind === K.PropertyDeclaration || node.kind === K.MethodDeclaration) return node.name.pos;
		const lastModifier = node.modifiers && node.modifiers.length ? node.modifiers[node.modifiers.length - 1] : undefined;
		return lastModifier && lastModifier.end >= 0 ? lastModifier.end : node.pos;
	}
	function emitClassDeclaration(node) {
		emitModifiers(node, node.modifiers);
		emitTokenWithComment(K.ClassKeyword, moveRangePastModifiersPos(node), writeKeyword, node);
		if (node.name) {
			writeSpace();
			emitIdentifierName(node.name);
		}
		const indented = f.getEmitFlags(node) & EmitFlags.Indented;
		if (indented) increaseIndent();
		emitTypeParameters(node, node.typeParameters);
		emitList(node, node.heritageClauses, ListFormat.ClassHeritageClauses);
		writeSpace();
		writePunctuation("{");
		emitList(node, node.members, ListFormat.ClassMembers);
		writePunctuation("}");
		if (indented) decreaseIndent();
	}
	function emitModuleDeclaration(node) {
		emitModifiers(node, node.modifiers);
		if (~node.flags & NodeFlags.GlobalAugmentation) {
			writeKeyword(node.flags & NodeFlags.Namespace ? "namespace" : "module");
			writeSpace();
		}
		emit(node.name);
		let body = node.body;
		if (!body) return writeTrailingSemicolon();
		while (body && body.kind === K.ModuleDeclaration) {
			writePunctuation(".");
			emit(body.name);
			body = body.body;
		}
		writeSpace();
		emit(body);
	}
	function isEmptyBlock(node) {
		return node.statements.length === 0 && (f.nodeIsSynthesized(node) || currentSourceFile === undefined || rangeIsOnSingleLine(node));
	}
	function emitBlockStatements(node, forceSingleLine) {
		emitTokenWithComment(K.OpenBraceToken, node.pos, writePunctuation, node);
		const format = forceSingleLine || f.getEmitFlags(node) & EmitFlags.SingleLine ? ListFormat.SingleLineBlockStatements : ListFormat.MultiLineBlockStatements;
		emitList(node, node.statements, format);
		emitTokenWithComment(K.CloseBraceToken, node.statements.end, writePunctuation, node, !!(format & ListFormat.MultiLine));
	}
	function emitImportEqualsDeclaration(node) {
		emitModifiers(node, node.modifiers);
		emitTokenWithComment(K.ImportKeyword, node.modifiers ? node.modifiers.end : node.pos, writeKeyword, node);
		writeSpace();
		if (node.isTypeOnly) {
			emitTokenWithComment(K.TypeKeyword, node.pos, writeKeyword, node);
			writeSpace();
		}
		emit(node.name);
		writeSpace();
		emitTokenWithComment(K.EqualsToken, node.name.end, writePunctuation, node);
		writeSpace();
		emit(node.moduleReference);
		writeTrailingSemicolon();
	}
	function emitImportDeclaration(node) {
		emitModifiers(node, node.modifiers);
		emitTokenWithComment(K.ImportKeyword, node.modifiers ? node.modifiers.end : node.pos, writeKeyword, node);
		writeSpace();
		if (node.importClause) {
			emit(node.importClause);
			writeSpace();
			emitTokenWithComment(K.FromKeyword, node.importClause.end, writeKeyword, node);
			writeSpace();
		}
		emitExpression(node.moduleSpecifier);
		if (node.attributes) {
			writeSpace();
			emit(node.attributes);
		}
		writeTrailingSemicolon();
	}
	function emitImportClause(node) {
		const phaseModifier = phaseModifierOf(node);
		if (phaseModifier !== undefined) {
			emitTokenWithComment(phaseModifier, node.pos, writeKeyword, node);
			writeSpace();
		}
		emit(node.name);
		if (node.name && node.namedBindings) {
			emitTokenWithComment(K.CommaToken, node.name.end, writePunctuation, node);
			writeSpace();
		}
		emit(node.namedBindings);
	}
	function emitExportAssignment(node) {
		const nextPos = emitTokenWithComment(K.ExportKeyword, node.pos, writeKeyword, node);
		writeSpace();
		if (node.isExportEquals) emitTokenWithComment(K.EqualsToken, nextPos, writeOperator, node);
		else emitTokenWithComment(K.DefaultKeyword, nextPos, writeKeyword, node);
		writeSpace();
		emitExpression(node.expression);
		writeTrailingSemicolon();
	}
	function emitExportDeclaration(node) {
		emitModifiers(node, node.modifiers);
		let nextPos = emitTokenWithComment(K.ExportKeyword, node.pos, writeKeyword, node);
		writeSpace();
		if (node.isTypeOnly) {
			nextPos = emitTokenWithComment(K.TypeKeyword, nextPos, writeKeyword, node);
			writeSpace();
		}
		if (node.exportClause) emit(node.exportClause);
		else nextPos = emitTokenWithComment(K.AsteriskToken, nextPos, writePunctuation, node);
		if (node.moduleSpecifier) {
			writeSpace();
			const fromPos = node.exportClause ? node.exportClause.end : nextPos;
			emitTokenWithComment(K.FromKeyword, fromPos, writeKeyword, node);
			writeSpace();
			emitExpression(node.moduleSpecifier);
		}
		if (node.attributes) {
			writeSpace();
			emit(node.attributes);
		}
		writeTrailingSemicolon();
	}
}

module.exports = { createPrinter, skipTrivia };
