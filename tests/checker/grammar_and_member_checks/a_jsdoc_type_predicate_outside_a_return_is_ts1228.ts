// @allowJs: true
// @checkJs: true
// A JSDoc `@type` is no return position: a type predicate there is TS1228,
// whether it types a cast or a variable.
// @filename: a.js
/** @type {number | string} */
var v = 1;
var r = /** @type {v is string} */(v === 1);
/** @type {v is string} */
var w;
