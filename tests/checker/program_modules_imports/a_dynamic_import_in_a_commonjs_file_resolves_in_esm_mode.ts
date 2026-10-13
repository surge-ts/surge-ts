// @module: node16
// @filename: package.json
{ "name": "pkg", "private": true }
// @filename: index.ts
export const value = 1;
// @filename: main.cts
import * as statically from "./";
void statically;
const dynamically = import("./");
const withIndex = import("./index");
const withExtension = import("./index.js");
