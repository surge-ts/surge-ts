"use strict";

// Source files and their syntax nodes, TypeScript-shaped, over the native
// node table. A node object is created the first time something reaches it
// and cached, so the same node is always the same object within its
// program; its kind, positions and flags read the native typed arrays, and
// its named children (`expression`, `arguments`, ...) are fetched per node
// on first access.
const native = require("./native.cjs");
const { SyntaxKind } = require("./enums.cjs");

const schema = native.nodeSchema();

function unsupported(method) {
	throw new Error(`surge-ts: ${method} is not supported; use ts.forEachChild to reach a node's children`);
}

/** tsc's `escapeLeadingUnderscores`. */
function escapeLeadingUnderscores(name) {
	return name.length >= 2 && name.charCodeAt(0) === 95 && name.charCodeAt(1) === 95 ? `_${name}` : name;
}

/** tsc's `unescapeLeadingUnderscores`. */
function unescapeLeadingUnderscores(name) {
	return name.length >= 3 && name.charCodeAt(0) === 95 && name.charCodeAt(1) === 95 && name.charCodeAt(2) === 95
		? name.slice(1)
		: name;
}

class NodeObject {
	constructor(sourceFile, index, kind) {
		this._sourceFile = sourceFile;
		this._index = index;
		this.kind = kind;
	}
	get pos() {
		return this._sourceFile._table.pos[this._index];
	}
	get end() {
		return this._sourceFile._table.end[this._index];
	}
	get flags() {
		return this._sourceFile._table.flags[this._index];
	}
	get parent() {
		const parent = this._sourceFile._table.parent[this._index];
		return parent < 0 ? undefined : this._sourceFile._node(parent);
	}
	getSourceFile() {
		return this._sourceFile;
	}
	getStart(sourceFile, includeJsDocComment) {
		const start = this._sourceFile._table.start[this._index];
		// Without a JSDoc comment before it, a node has none attached, and its
		// start is the same either way.
		if (includeJsDocComment && this._sourceFile.text.substring(this.pos, start).includes("/**")) {
			throw new Error("surge-ts: Node.getStart with includeJsDocComment is not supported yet for a node after a JSDoc comment");
		}
		return start;
	}
	getFullStart() {
		return this.pos;
	}
	getEnd() {
		return this.end;
	}
	getWidth() {
		return this.end - this.getStart();
	}
	getFullWidth() {
		return this.end - this.pos;
	}
	getLeadingTriviaWidth() {
		return this.getStart() - this.pos;
	}
	getFullText() {
		return this._sourceFile.text.substring(this.pos, this.end);
	}
	getText() {
		return this._sourceFile.text.substring(this.getStart(), this.end);
	}
	forEachChild(cbNode, cbNodes) {
		return forEachChild(this, cbNode, cbNodes);
	}
	getChildCount() {
		return unsupported("Node.getChildCount");
	}
	getChildAt() {
		return unsupported("Node.getChildAt");
	}
	getChildren() {
		return unsupported("Node.getChildren");
	}
	getFirstToken() {
		return unsupported("Node.getFirstToken");
	}
	getLastToken() {
		return unsupported("Node.getLastToken");
	}
}

const TEXT_KINDS = new Set([
	SyntaxKind.Identifier,
	SyntaxKind.PrivateIdentifier,
	SyntaxKind.StringLiteral,
	SyntaxKind.NumericLiteral,
	SyntaxKind.BigIntLiteral,
	SyntaxKind.RegularExpressionLiteral,
	SyntaxKind.NoSubstitutionTemplateLiteral,
	SyntaxKind.TemplateHead,
	SyntaxKind.TemplateMiddle,
	SyntaxKind.TemplateTail,
	SyntaxKind.JsxText,
]);

/** Scalar (non-node) properties per kind: `[name, read(scalars)]`. */
const SCALARS = new Map();
for (const kind of TEXT_KINDS) SCALARS.set(kind, [["text", (scalars) => scalars.text ?? ""]]);
for (const kind of [SyntaxKind.Identifier, SyntaxKind.PrivateIdentifier]) {
	SCALARS.get(kind).push(["escapedText", (scalars) => escapeLeadingUnderscores(scalars.text ?? "")]);
}
for (const kind of [SyntaxKind.PrefixUnaryExpression, SyntaxKind.PostfixUnaryExpression, SyntaxKind.TypeOperator]) {
	SCALARS.set(kind, [["operator", (scalars) => scalars.operator]]);
}
for (const kind of [SyntaxKind.HeritageClause, SyntaxKind.ImportAttributes]) {
	SCALARS.set(kind, [["token", (scalars) => scalars.operator]]);
}
SCALARS.set(SyntaxKind.MetaProperty, [["keywordToken", (scalars) => scalars.operator]]);
SCALARS.get(SyntaxKind.JsxText).push(["containsOnlyTriviaWhiteSpaces", (scalars) => jsxTextIsTrivia(scalars.text ?? "")]);
for (const kind of [
	SyntaxKind.ImportClause,
	SyntaxKind.ImportEqualsDeclaration,
	SyntaxKind.ImportSpecifier,
	SyntaxKind.ExportSpecifier,
	SyntaxKind.ExportDeclaration,
]) {
	SCALARS.set(kind, [["isTypeOnly", (scalars) => scalars.isTypeOnly]]);
}
SCALARS.set(SyntaxKind.ImportType, [["isTypeOf", (scalars) => scalars.isTypeOnly]]);
SCALARS.set(SyntaxKind.ExportAssignment, [["isExportEquals", (scalars) => scalars.isExportEquals]]);

/**
 * tsc's scanner (`scanJsxToken`): JSX text is trivia when a line break comes
 * before anything but whitespace, and nothing but whitespace follows.
 */
function jsxTextIsTrivia(text) {
	let state = 0;
	for (const character of text) {
		const code = character.codePointAt(0);
		const lineBreak = code === 0x0a || code === 0x0d || code === 0x2028 || code === 0x2029;
		if (lineBreak && state === 0) state = -1;
		else if (!lineBreak && !isWhiteSpaceSingleLineCode(code)) state = 1;
	}
	return state === -1;
}

function isWhiteSpaceSingleLineCode(code) {
	return (
		code === 0x20 ||
		code === 0x09 ||
		code === 0x0b ||
		code === 0x0c ||
		code === 0xa0 ||
		code === 0x85 ||
		code === 0x1680 ||
		(code >= 0x2000 && code <= 0x200b) ||
		code === 0x202f ||
		code === 0x205f ||
		code === 0x3000 ||
		code === 0xfeff
	);
}

const nodeClasses = new Map();

/** The node class for a kind: its named children and scalars as getters. */
function nodeClass(kind, base = NodeObject) {
	const key = base === NodeObject ? kind : `${kind}:${base.name}`;
	let cls = nodeClasses.get(key);
	if (cls) return cls;
	cls = class extends base {};
	for (const [name, isList] of schema[kind] ?? []) {
		Object.defineProperty(cls.prototype, name, {
			get() {
				return this._sourceFile._childProperty(this._index, name, isList);
			},
			enumerable: true,
			configurable: true,
		});
	}
	for (const [name, read] of SCALARS.get(kind) ?? []) {
		Object.defineProperty(cls.prototype, name, {
			get() {
				return read(this._sourceFile._scalars(this._index));
			},
			enumerable: true,
			configurable: true,
		});
	}
	nodeClasses.set(key, cls);
	return cls;
}

/**
 * A source file: TypeScript's `SourceFile` node over a backing that answers
 * `nodeTable()`, `nodeProperties(index)`, `nodeScalars(index)`, `text()`
 * and `lineStarts()` for one file.
 */
class SourceFileObject extends nodeClass(SyntaxKind.SourceFile) {
	constructor(backing, info, program, fileIndex) {
		const table = backing.nodeTable();
		super(undefined, table.root, SyntaxKind.SourceFile);
		this._sourceFile = this;
		this._backing = backing;
		this._table = table;
		this._program = program;
		this._fileIndex = fileIndex;
		this._nodes = new Array(table.kinds.length);
		this._nodes[table.root] = this;
		this._properties = new Map();
		this._scalarCache = new Map();
		this._lists = new Map();
		this._text = undefined;
		this._lineStarts = undefined;
		this.fileName = info.fileName;
		this.languageVersion = info.languageVersion;
		this.languageVariant = info.languageVariant;
		this.scriptKind = info.scriptKind;
		this.isDeclarationFile = info.isDeclarationFile;
	}
	get text() {
		if (this._text === undefined) this._text = this._backing.text();
		return this._text;
	}
	getSourceFile() {
		return this;
	}
	/** @internal The node with this index, created once. */
	_node(index) {
		let node = this._nodes[index];
		if (node === undefined) {
			const kind = this._table.kinds[index];
			node = new (nodeClass(kind))(this, index, kind);
			this._nodes[index] = node;
		}
		return node;
	}
	_record(index) {
		let record = this._properties.get(index);
		if (record === undefined) {
			record = new Map(this._backing.nodeProperties(index).map((property) => [property.name, property]));
			this._properties.set(index, record);
		}
		return record;
	}
	_childProperty(index, name, isList) {
		const property = this._record(index).get(name);
		if (property === undefined) return undefined;
		if (!isList) return this._node(property.node);
		const key = `${index}:${name}`;
		let list = this._lists.get(key);
		if (list === undefined) {
			list = property.nodes.map((child) => this._node(child));
			list.pos = property.pos;
			list.end = property.end;
			const text = () => this.text;
			Object.defineProperty(list, "hasTrailingComma", {
				get() {
					if (this.length === 0) return false;
					const source = text();
					let position = this[this.length - 1].end;
					while (position < this.end && /\s/.test(source[position])) position++;
					return source[position] === ",";
				},
			});
			this._lists.set(key, list);
		}
		return list;
	}
	_scalars(index) {
		let scalars = this._scalarCache.get(index);
		if (scalars === undefined) {
			scalars = this._backing.nodeScalars(index);
			this._scalarCache.set(index, scalars);
		}
		return scalars;
	}
	getLineStarts() {
		if (this._lineStarts === undefined) this._lineStarts = Array.from(this._backing.lineStarts());
		return this._lineStarts;
	}
	getLineAndCharacterOfPosition(position) {
		return computeLineAndCharacterOfPosition(this.getLineStarts(), position);
	}
	getPositionOfLineAndCharacter(line, character, allowEdits) {
		return computePositionOfLineAndCharacter(this.getLineStarts(), line, character, this.text, allowEdits);
	}
	getLineEndOfPosition(position) {
		const { line } = this.getLineAndCharacterOfPosition(position);
		const lineStarts = this.getLineStarts();
		let lastCharPos;
		if (line + 1 >= lineStarts.length) lastCharPos = this.end;
		else lastCharPos = lineStarts[line + 1] - 1;
		const text = this.text;
		if (lastCharPos > 0 && lastCharPos < text.length && (text[lastCharPos] === "\n" || text[lastCharPos] === "\r")) {
			return text[lastCharPos - 1] === "\r" ? lastCharPos - 1 : lastCharPos;
		}
		return lastCharPos;
	}
}

/** tsc's `computeLineAndCharacterOfPosition`. */
function computeLineAndCharacterOfPosition(lineStarts, position) {
	let low = 0;
	let high = lineStarts.length - 1;
	while (low <= high) {
		const middle = (low + high) >>> 1;
		if (lineStarts[middle] === position) return { line: middle, character: 0 };
		if (lineStarts[middle] < position) low = middle + 1;
		else high = middle - 1;
	}
	const line = low - 1;
	if (line < 0) throw new Error("position cannot precede the beginning of the file");
	return { line, character: position - lineStarts[line] };
}

/** tsc's `computePositionOfLineAndCharacter`. */
function computePositionOfLineAndCharacter(lineStarts, line, character, text, allowEdits) {
	if (line < 0 || line >= lineStarts.length) {
		if (allowEdits) line = line < 0 ? 0 : line >= lineStarts.length ? lineStarts.length - 1 : line;
		else throw new Error(`Bad line number. Line: ${line}, lineStarts.length: ${lineStarts.length}`);
	}
	const position = lineStarts[line] + character;
	if (allowEdits) {
		return position > lineStarts[line + 1] ? lineStarts[line + 1] : typeof text === "string" && position > text.length ? text.length : position;
	}
	if (line < lineStarts.length - 1) {
		if (position >= lineStarts[line + 1]) throw new Error("Computed position is beyond that of the following line.");
	} else if (text !== undefined && position > text.length) {
		throw new Error("Computed position is beyond the end of the file.");
	}
	return position;
}

/** tsc's `computeLineStarts`. */
function computeLineStarts(text) {
	const result = [];
	let position = 0;
	let lineStart = 0;
	while (position < text.length) {
		const char = text.charCodeAt(position);
		position++;
		if (char === 13) {
			if (text.charCodeAt(position) === 10) position++;
			result.push(lineStart);
			lineStart = position;
		} else if (char === 10 || char === 0x2028 || char === 0x2029) {
			result.push(lineStart);
			lineStart = position;
		}
	}
	result.push(lineStart);
	return result;
}

function ownNode(node) {
	if (!(node instanceof NodeObject)) throw new TypeError("surge-ts: expected a node from a surge-ts source file");
	return node;
}

/**
 * tsc's `forEachChild`: calls `cbNode` for each child node, or `cbNodes` for
 * each `NodeArray` when given, in tsc's order; stops at the first truthy
 * result and returns it.
 */
function forEachChild(node, cbNode, cbNodes) {
	if (node === undefined || node.kind <= SyntaxKind.LastToken) return undefined;
	ownNode(node);
	const sourceFile = node._sourceFile;
	if (cbNodes === undefined) {
		const table = sourceFile._table;
		const index = node._index;
		const end = table.childOffsets[index + 1];
		for (let offset = table.childOffsets[index]; offset < end; offset++) {
			const result = cbNode(sourceFile._node(table.children[offset]));
			if (result) return result;
		}
		return undefined;
	}
	for (const [name, isList] of schema[node.kind] ?? []) {
		const value = node[name];
		if (value === undefined) continue;
		const result = isList ? cbNodes(value) : cbNode(value);
		if (result) return result;
	}
	return undefined;
}

module.exports = {
	NodeObject,
	SourceFileObject,
	forEachChild,
	computeLineStarts,
	computeLineAndCharacterOfPosition,
	computePositionOfLineAndCharacter,
	escapeLeadingUnderscores,
	unescapeLeadingUnderscores,
	ownNode,
};
