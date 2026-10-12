// @allowJs: true
// @checkJs: true
// @module: commonjs
// @filename: index.js
/** @type {typeof import("./nonexistent")} */
const a = [];
/** @type {import("./nonexistent2").X} */
const b = 1;
module.exports = { a, b };
