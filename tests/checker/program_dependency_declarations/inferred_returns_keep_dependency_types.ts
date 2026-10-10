// An unannotated function returning a dependency's type keeps it after the
// environment the reference was resolved in is gone.
// @filename: node_modules/pk/package.json
{ "name": "pk", "types": "index.d.ts" }
// @filename: node_modules/pk/index.d.ts
export interface IBox<T> { v: T }
export type Box<T> = { v: T };
// @filename: src/index.ts
import type { Box, IBox } from "pk";
declare const ib: IBox<string>;
declare const b: Box<string>;
function interfaceReturn() { return ib; }
function aliasReturn() { return b; }
const n1: number = interfaceReturn();
const n2: number = aliasReturn();
interfaceReturn().v.toFixed();
