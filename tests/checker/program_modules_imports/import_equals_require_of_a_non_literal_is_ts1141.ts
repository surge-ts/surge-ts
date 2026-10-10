// @module: commonjs
// tsc parses any expression in `require(…)` and its checker wants a string
// literal; the file is still checked, and nothing is resolved.
var name = "filename";
import foo = require(name);
const n: number = "s";
