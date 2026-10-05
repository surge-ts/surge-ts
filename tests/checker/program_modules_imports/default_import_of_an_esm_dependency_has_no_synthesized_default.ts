// @target: esnext
// @module: preserve
// @moduleResolution: bundler
// @filename: node_modules/esm-dep/package.json
{ "type": "module" }
// @filename: node_modules/esm-dep/index.d.ts
export function toString(): string;
// @filename: node_modules/cjs-dep/package.json
{}
// @filename: node_modules/cjs-dep/index.d.ts
export function toString(): string;
// @filename: index.ts
import esm, { toString } from 'esm-dep';
import cjs from 'cjs-dep';
esm;
cjs;
toString();
