"use strict";

// `Node.getChildren()` and the token API over it (`getChildCount`,
// `getChildAt`, `getFirstToken`, `getLastToken`): TypeScript's
// `createChildren`, which interleaves a node's children with the tokens
// scanned between them and wraps each list in a `SyntaxList`.
//
// The text between children is trivia, punctuation and keywords only (every
// literal and name is a node of its own), so a scanner of those is enough to
// produce the tokens TypeScript's scanner does there.

const { SyntaxKind } = require("./enums.cjs");
const { NodeObject, forEachChild } = require("./nodes.cjs");
const { isLineBreak, isWhiteSpaceLike } = require("./utilities.cjs");

const K = SyntaxKind;
const tokenKinds = require("./token-texts.json");
// TypeScript's scanner never joins `>` with what follows (the parser
// rescans it) and, outside JSX mode, scans `</` as two tokens.
const punctuation = Object.keys(tokenKinds)
	.filter((text) => !/^[a-z]/.test(text) && !(text.startsWith(">") && text.length > 1) && text !== "</")
	.sort((a, b) => b.length - a.length);
const identifierStart = /[\p{ID_Start}$_\\]/u;
const identifierPart = /[\p{ID_Continue}$‌‍]/u;

/** The position after trivia (whitespace and comments) at `pos`. */
function skipTrivia(text, pos) {
	while (pos < text.length) {
		const ch = text.charCodeAt(pos);
		if (isWhiteSpaceLike(ch)) {
			pos++;
		} else if (ch === 47 && text.charCodeAt(pos + 1) === 47) {
			pos += 2;
			while (pos < text.length && !isLineBreak(text.charCodeAt(pos))) pos++;
		} else if (ch === 47 && text.charCodeAt(pos + 1) === 42) {
			const close = text.indexOf("*/", pos + 2);
			pos = close < 0 ? text.length : close + 2;
		} else if (ch === 35 && pos === 0 && text.charCodeAt(1) === 33) {
			while (pos < text.length && !isLineBreak(text.charCodeAt(pos))) pos++;
		} else {
			break;
		}
	}
	return pos;
}

/** The token at `start` (after trivia): its kind and end. */
function scanToken(text, start) {
	if (start >= text.length) return { kind: K.EndOfFileToken, end: start };
	const char = String.fromCodePoint(text.codePointAt(start));
	if (identifierStart.test(char)) {
		let end = start + char.length;
		while (end < text.length) {
			const next = String.fromCodePoint(text.codePointAt(end));
			if (!identifierPart.test(next)) break;
			end += next.length;
		}
		const word = text.slice(start, end);
		return { kind: tokenKinds[word] ?? K.Identifier, end };
	}
	for (const candidate of punctuation) {
		if (!text.startsWith(candidate, start)) continue;
		// `?.` before a digit is `?` and a numeric literal.
		if (candidate === "?." && /\d/.test(text[start + 2] ?? "")) continue;
		return { kind: tokenKinds[candidate], end: start + candidate.length };
	}
	return { kind: K.Unknown, end: start + char.length };
}

class TokenObject {
	constructor(kind, pos, end, parent) {
		this.kind = kind;
		this.pos = pos;
		this.end = end;
		this.parent = parent;
		this.flags = 0;
	}
	getSourceFile() {
		return this.parent.getSourceFile();
	}
	getStart(sourceFile) {
		return skipTrivia((sourceFile ?? this.getSourceFile()).text, this.pos);
	}
	getFullStart() {
		return this.pos;
	}
	getEnd() {
		return this.end;
	}
	getWidth(sourceFile) {
		return this.end - this.getStart(sourceFile);
	}
	getFullWidth() {
		return this.end - this.pos;
	}
	getLeadingTriviaWidth(sourceFile) {
		return this.getStart(sourceFile) - this.pos;
	}
	getFullText(sourceFile) {
		return (sourceFile ?? this.getSourceFile()).text.substring(this.pos, this.end);
	}
	getText(sourceFile) {
		return (sourceFile ?? this.getSourceFile()).text.substring(this.getStart(sourceFile), this.end);
	}
	getChildCount() {
		return 0;
	}
	getChildAt() {
		return undefined;
	}
	getChildren() {
		return [];
	}
	getFirstToken() {
		return undefined;
	}
	getLastToken() {
		return undefined;
	}
	forEachChild() {
		return undefined;
	}
}

/** tsc's `addSyntheticNodes`: the tokens between `pos` and `end`. */
function addTokens(children, pos, end, parent, text) {
	while (pos < end) {
		const start = skipTrivia(text, pos);
		const { kind, end: tokenEnd } = scanToken(text, start);
		if (tokenEnd <= end) children.push(new TokenObject(kind, pos, tokenEnd, parent));
		pos = tokenEnd;
		if (kind === K.EndOfFileToken) break;
	}
}

class SyntaxListObject extends TokenObject {
	constructor(nodes, parent, text) {
		super(K.SyntaxList, nodes.pos, nodes.end, parent);
		this._children = [];
		let pos = nodes.pos;
		for (const node of nodes) {
			addTokens(this._children, pos, node.pos, parent, text);
			this._children.push(node);
			pos = node.end;
		}
		addTokens(this._children, pos, nodes.end, parent, text);
	}
	getChildren() {
		return this._children;
	}
	getChildCount() {
		return this._children.length;
	}
	getChildAt(index) {
		return this._children[index];
	}
	getFirstToken(sourceFile) {
		return firstToken(this, sourceFile);
	}
	getLastToken(sourceFile) {
		return lastToken(this, sourceFile);
	}
}

const childrenCache = new WeakMap();

/** tsc's `createChildren`. */
function getChildren(node, sourceFile) {
	// A token (an identifier, a keyword, a literal) has no children.
	if (node.kind < K.FirstNode) return [];
	let children = childrenCache.get(node);
	if (children) return children;
	const text = (sourceFile ?? node.getSourceFile()).text;
	children = [];
	let pos = node.pos;
	forEachChild(
		node,
		(child) => {
			addTokens(children, pos, child.pos, node, text);
			children.push(child);
			pos = child.end;
		},
		(nodes) => {
			addTokens(children, pos, nodes.pos, node, text);
			children.push(new SyntaxListObject(nodes, node, text));
			pos = nodes.end;
		},
	);
	addTokens(children, pos, node.end, node, text);
	childrenCache.set(node, children);
	return children;
}

function firstToken(node, sourceFile) {
	const child = node.getChildren(sourceFile)[0];
	if (!child) return undefined;
	return child.kind < K.FirstNode ? child : child.getFirstToken(sourceFile);
}

function lastToken(node, sourceFile) {
	const children = node.getChildren(sourceFile);
	const child = children[children.length - 1];
	if (!child) return undefined;
	return child.kind < K.FirstNode ? child : child.getLastToken(sourceFile);
}

Object.assign(NodeObject.prototype, {
	getChildren(sourceFile) {
		return getChildren(this, sourceFile);
	},
	getChildCount(sourceFile) {
		return getChildren(this, sourceFile).length;
	},
	getChildAt(index, sourceFile) {
		return getChildren(this, sourceFile)[index];
	},
	getFirstToken(sourceFile) {
		return firstToken(this, sourceFile);
	},
	getLastToken(sourceFile) {
		return lastToken(this, sourceFile);
	},
});

module.exports = { TokenObject, SyntaxListObject };
