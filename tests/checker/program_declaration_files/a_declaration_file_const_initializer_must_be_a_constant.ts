// @filename: local.d.ts
export let a = 1;
export const b: number = 1;
export const c = 1 + 1;
export const d = "s";
export const e = -1;
export const f = `t`;
export declare class K { x = 1; readonly y = 2; readonly z = 1 + 1; }
export declare namespace N { const q = 2 * 2; }
// @filename: node_modules/y/index.d.ts
export const g = 1 + 1;
export const h = 1;
// @filename: node_modules/y/package.json
{ "name": "y", "version": "1.0.0" }
// @filename: a.ts
import "y";
export {};
