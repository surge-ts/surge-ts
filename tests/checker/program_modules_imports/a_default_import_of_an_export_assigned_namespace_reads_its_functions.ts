// @strict: true
// @esModuleInterop: true
// @module: esnext
// @moduleResolution: bundler
// @filename: node_modules/tslike/package.json
{ "name": "tslike", "version": "1.0.0", "types": "index.d.ts" }
// @filename: node_modules/tslike/index.d.ts
declare namespace tslike {
  interface Node { kind: number }
  interface Identifier extends Node { text: string }
  function isIdentifier(node: Node): node is Identifier;
  function create(name: string): Node;
}
export = tslike;
// @filename: index.ts
// The default import is the assigned namespace, whose functions keep their
// signatures: arities are checked and predicates narrow.
import ts from "tslike";
export function name(node: ts.Node): string {
  if (ts.isIdentifier(node)) return node.text;
  return "";
}
ts.isIdentifier(ts.create("a"), 1);
