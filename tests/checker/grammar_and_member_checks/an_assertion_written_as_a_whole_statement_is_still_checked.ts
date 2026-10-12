// @strict: true
// @module: commonjs
// @allowJs: true
// @checkJs: true
// @filename: a.ts
type FooFun = (foo: number) => string;
undefined as FooFun;
function f() { undefined as FooFun; }
export = undefined as FooFun;
// @filename: b.js
/** @typedef {(foo: number) => string} FooFun */
module.exports = /** @type {FooFun} */(void 0);
