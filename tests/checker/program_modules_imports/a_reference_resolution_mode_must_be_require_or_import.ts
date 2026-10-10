// @module: node16
/// <reference types="pkg" resolution-mode="esm"/>
// tsc's parser reports a `resolution-mode` other than `require` or `import`
// on a `types` reference.
export {};
// @filename: node_modules/pkg/package.json
{ "name": "pkg", "version": "0.0.1", "types": "./index.d.ts" }
// @filename: node_modules/pkg/index.d.ts
declare var fromPkg: number;
