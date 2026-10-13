// tsc's `isJSLiteralType` (noImplicitAny off): a member a JavaScript object
// literal does not declare reads as `any`, but the literal is related like any
// other object, so a required property it lacks is still missing.
// @allowJs: true
// @checkJs: true
// @strict: false
// @filename: a.js
const o = { a: 12 };
o.zzz;
/** @type {{ a: number, b: string }} */
const p = o;
/** @param {{ a: number, b: string }} t */
function q(t) {}
q(o);
q({ a: 1 });
/**
 * @template {{ a: number, b: string }} T
 * @param {T} t
 */
function g(t) {}
g({ a: 12 });
/** @type {{ a: number }} */
let widened = { a: 1, extra: 2 };
/**
 * @template T
 * @param {T} t
 * @returns {T}
 */
function id(t) { return t; }
id({ a: 1 }).zzz;
