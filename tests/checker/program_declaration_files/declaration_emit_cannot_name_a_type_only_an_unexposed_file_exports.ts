// @declaration: true
// @module: nodenext
// @filename: node_modules/pkg/package.json
{ "name": "pkg", "exports": { ".": "./index.js" } }
// @filename: node_modules/pkg/index.d.ts
export { make, makeAlias } from "./internal.js";
// @filename: node_modules/pkg/internal.d.ts
export interface Hidden { x: number }
type Written = { y: string };
export declare function make(): Hidden;
export declare function makeAlias(): Written;
// @filename: src/index.ts
import { make, makeAlias } from "pkg";
export const value = make();
export const written = makeAlias();
export default make();
