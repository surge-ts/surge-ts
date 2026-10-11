// @allowJs: true
// @checkJs: true
// `exports.C = C` is an alias of the class, type meaning included, so the
// JSDoc import type names the class.
// @filename: mod1.js
class C {
    s() { }
}
module.exports.C = C
// @filename: test.js
/** @typedef {import('./mod1').C} X */
/** @param {X} c */
function demo(c) {
    c.s
}
