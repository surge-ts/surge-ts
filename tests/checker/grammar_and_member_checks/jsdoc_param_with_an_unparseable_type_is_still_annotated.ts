// @allowJs: true
// @checkJs: true
// @noImplicitAny: true
// @filename: a.js
/** @param {???!?number?=} a */
function g(a) {
    a = 1;
    a = null;
}
g();
g(null);
/** @param {number} y */
function h(y, z) {}
