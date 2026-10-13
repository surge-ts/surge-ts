// @target: es2022
// @module: node18
// @rewriteRelativeImportExtensions: true
// @verbatimModuleSyntax: true
// @filename: package.json
{
  "name": "pkg",
  "type": "module",
  "imports": {
    "#foo.ts": "./foo.ts",
    "#internal/*": "./internal/*"
  }
}
// @filename: foo.ts
export {};
// @filename: internal/foo.ts
export {};
// @filename: index.ts
// `rewriteRelativeImportExtensions` rewrites only relative specifiers: a
// package import resolved through the `.ts` it writes keeps it (TS2877).
import {} from "#foo.ts";
import {} from "#internal/foo.ts";
