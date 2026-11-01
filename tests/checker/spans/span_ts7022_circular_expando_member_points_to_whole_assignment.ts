// @allowJs: true
// @checkJs: true
// @noImplicitAny: true
// @surge-compare: spans
// @filename: expando.js
var test = {};
test.K = test.K || function () {};
test.L = (test.L || 1);
function local() {
    var o = {};
    o.M = o.M || 3;
}
var t = {};
t.P = /** @type {any} */ (t.P || 4);
var o = {};
/** c */ o.N = o.N || 1 /* t */;
function F() {}
F.prototype = F.prototype || {};
