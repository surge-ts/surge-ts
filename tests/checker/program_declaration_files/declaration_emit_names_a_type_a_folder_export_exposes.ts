// @declaration: true
// @module: nodenext
// @filename: node_modules/inner/package.json
{ "name": "inner", "exports": { "./": "./" } }
// @filename: node_modules/inner/other.d.ts
export interface Thing { x: number }
export declare function make(): Thing;
// @filename: src/index.ts
import { make } from "inner/other.js";
export const value = make();
