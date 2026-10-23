"use strict";

// tsc's `visitNode`, `visitNodes` and `visitEachChild` (visitorPublic.ts)
// over the nodes declarations keep: a list or node nothing in changes is
// returned as it is; a changed list keeps the original's text range.
const { SyntaxKind: K } = require("../enums.cjs");
const native = require("../native.cjs");
const factory = require("./factory.cjs");

const schema = native.nodeSchema();

// Children tsc visits with its token visitor, which declaration emit leaves
// unset: they are kept as they are.
const TOKEN_CHILDREN = new Set([
	"questionToken",
	"exclamationToken",
	"dotDotDotToken",
	"asteriskToken",
	"readonlyToken",
	"questionDotToken",
	"equalsGreaterThanToken",
	"endOfFileToken",
	"assertsModifier",
]);

function visitNode(node, visitor) {
	if (node === undefined) return undefined;
	const visited = visitor(node);
	if (visited === node || visited === undefined) return visited;
	if (Array.isArray(visited)) return visited.length === 1 ? visited[0] : undefined;
	return visited;
}

function visitArrayWorker(nodes, visitor, start, count) {
	let updated;
	if (start > 0 || count < nodes.length) updated = [];
	for (let i = 0; i < count; i++) {
		const node = nodes[i + start];
		const visited = node !== undefined ? (visitor ? visitor(node) : node) : undefined;
		if (updated !== undefined || visited === undefined || visited !== node) {
			if (updated === undefined) updated = nodes.slice(0, i);
			if (visited) {
				if (Array.isArray(visited)) for (const each of visited) updated.push(each);
				else updated.push(visited);
			}
		}
	}
	return updated ?? nodes;
}

/** tsc's `visitArray`: a plain array, or `nodes` itself when nothing changes. */
function visitArray(nodes, visitor) {
	if (nodes === undefined) return undefined;
	return visitArrayWorker(nodes, visitor, 0, nodes.length);
}

/** tsc's `visitNodes`. */
function visitNodes(nodes, visitor) {
	if (nodes === undefined) return undefined;
	const updated = visitArrayWorker(nodes, visitor, 0, nodes.length);
	if (updated === nodes) return nodes;
	const array = factory.createNodeArray(updated, nodes.hasTrailingComma);
	array.pos = nodes.pos;
	array.end = nodes.end;
	return array;
}

/** tsc's `visitEachChild` for the nodes a declaration file holds. */
function visitEachChild(node, visitor) {
	if (node === undefined) return undefined;
	const children = schema[node.kind];
	if (!children || children.length === 0) return node;
	const changes = {};
	for (const [name, isList] of children) {
		const child = node[name];
		if (TOKEN_CHILDREN.has(name) || child === undefined) changes[name] = child;
		else changes[name] = isList ? visitNodes(child, visitor) : visitNode(child, visitor);
	}
	return factory.updateNode(node, changes);
}

module.exports = { visitNode, visitNodes, visitArray, visitEachChild };
