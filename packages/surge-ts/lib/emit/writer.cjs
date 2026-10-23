"use strict";

// tsc's `createTextWriter`: text with four-space indentation applied at the
// start of each line, tracking the line and column source maps record.

/** tsc's `computeLineStarts`: CR, LF, CRLF, U+2028 and U+2029 end a line. */
function computeLineStarts(text) {
	const result = [0];
	let lineStart = 0;
	for (let position = 0; position < text.length; ) {
		const ch = text.charCodeAt(position);
		position++;
		if (ch === 13) {
			if (text.charCodeAt(position) === 10) position++;
			result.push(lineStart = position);
		} else if (ch === 10 || ch === 0x2028 || ch === 0x2029) {
			result.push(lineStart = position);
		}
	}
	return result;
}

function createTextWriter(newLine) {
	let output;
	let indent;
	let lineStart;
	let lineCount;
	let linePos;
	let hasTrailingComment = false;

	function updateLineCountAndPosFor(s) {
		const lineStartsOfS = computeLineStarts(s);
		if (lineStartsOfS.length > 1) {
			lineCount = lineCount + lineStartsOfS.length - 1;
			linePos = output.length - s.length + lineStartsOfS[lineStartsOfS.length - 1];
			lineStart = linePos - output.length === 0;
		} else {
			lineStart = false;
		}
	}
	function writeText(s) {
		if (s && s.length) {
			if (lineStart) {
				s = "    ".repeat(indent) + s;
				lineStart = false;
			}
			output += s;
			updateLineCountAndPosFor(s);
		}
	}
	function write(s) {
		if (s) hasTrailingComment = false;
		writeText(s);
	}
	function writeComment(s) {
		if (s) hasTrailingComment = true;
		writeText(s);
	}
	function reset() {
		output = "";
		indent = 0;
		lineStart = true;
		lineCount = 0;
		linePos = 0;
		hasTrailingComment = false;
	}
	function rawWrite(s) {
		if (s !== undefined) {
			output += s;
			updateLineCountAndPosFor(s);
			hasTrailingComment = false;
		}
	}
	function writeLine(force) {
		if (!lineStart || force) {
			output += newLine;
			lineCount++;
			linePos = output.length;
			lineStart = true;
			hasTrailingComment = false;
		}
	}
	reset();
	return {
		write,
		rawWrite,
		writeLiteral: (s) => {
			if (s && s.length) write(s);
		},
		writeLine,
		increaseIndent: () => {
			indent++;
		},
		decreaseIndent: () => {
			indent--;
		},
		getIndent: () => indent,
		getTextPos: () => output.length,
		getLine: () => lineCount,
		getColumn: () => (lineStart ? indent * 4 : output.length - linePos),
		getText: () => output,
		isAtStartOfLine: () => lineStart,
		hasTrailingComment: () => hasTrailingComment,
		hasTrailingWhitespace: () => !!output.length && /\s/.test(output[output.length - 1]),
		clear: reset,
		writeKeyword: write,
		writeOperator: write,
		writeParameter: write,
		writeProperty: write,
		writePunctuation: write,
		writeSpace: write,
		writeStringLiteral: write,
		writeSymbol: (s) => write(s),
		writeTrailingSemicolon: write,
		writeComment,
	};
}

module.exports = { createTextWriter, computeLineStarts };
