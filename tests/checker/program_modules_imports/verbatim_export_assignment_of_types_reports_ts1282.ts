// @verbatimModuleSyntax: true
// @target: esnext
// @module: commonjs
// @moduleResolution: bundler
// @noEmit: true
// @filename: a.ts
interface I {}
export = I;
// @filename: b.ts
import I = require("./a");
// @filename: c.ts
interface I {}
namespace I {
    export const x = 1;
}
export = I;
// @filename: d.ts
import I = require("./c");
import type J = require("./c");
export = J;
// @filename: e.d.ts
interface I {}
export = I;
// @filename: f.ts
import type I = require("./e");
const I = {};
export = I;
// @filename: g.ts
type T = number;
export = T;
