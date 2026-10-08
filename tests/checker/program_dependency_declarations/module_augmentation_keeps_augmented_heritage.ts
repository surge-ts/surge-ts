// @filename: node_modules/t/package.json
{ "name": "t", "types": "index.d.ts" }
// @filename: node_modules/t/gen/ast.d.ts
export declare interface NodeOrTokenData { type: string; loc: number; range: [number, number]; }
export declare interface BaseNode extends NodeOrTokenData { type: string; }
export declare interface SpreadElement extends BaseNode { type: "S"; argument: Node; }
export declare interface ArrayExpression extends BaseNode { type: "A"; elements: Node[]; }
export declare type Node = SpreadElement | ArrayExpression;
// @filename: node_modules/t/est.d.ts
import type * as TSESTree from './gen/ast';
declare module './gen/ast' {
    interface BaseNode { parent: TSESTree.Node; }
    interface SpreadElement { parent: TSESTree.ArrayExpression; }
}
export * as TSESTree from './gen/ast';
// @filename: node_modules/t/index.d.ts
export * from './est';
// @filename: src/index.ts
import type { TSESTree } from 't';
export const base: 1 = null! as TSESTree.BaseNode;
declare const spread: TSESTree.SpreadElement;
const range: [number, number] = spread.range;
const parent: TSESTree.ArrayExpression = spread.parent;
