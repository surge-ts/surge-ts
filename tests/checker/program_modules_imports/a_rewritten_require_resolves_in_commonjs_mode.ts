// @target: es2022
// @module: node18
// @rewriteRelativeImportExtensions: true
// @verbatimModuleSyntax: true
// An ES module's `import x = require()` still resolves the way `require`
// does, through `./foo.ts/index.ts`, which rewriting would miss (TS2876).
// @filename: package.json
{ "type": "module" }
// @filename: foo.ts/index.ts
export = {};
// @filename: index.ts
import foo = require("./foo.ts");
import type _foo = require("./foo.ts");
