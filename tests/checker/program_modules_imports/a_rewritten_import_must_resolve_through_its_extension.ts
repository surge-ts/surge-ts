// @target: es2022
// @module: node18
// @rewriteRelativeImportExtensions: true
// `rewriteRelativeImportExtensions` rewrites `./foo.ts` to `./foo.js`; one
// that resolved to `./foo.ts/index.ts` instead would then name a file that
// is not there (TS2876). A type-only import is never emitted.
// @filename: foo.ts/index.ts
export = {};
// @filename: index.ts
import foo = require("./foo.ts");
import type _foo = require("./foo.ts");
