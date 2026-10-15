// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: a.js
var Outer = {};
Outer.Fn = function (/** @type {number} */ n) { return n; };
Outer.Fn.Message = "m";
Outer.Fn.Count = 1;
/** @type {string} */
var s = Outer.Fn.Message;
/** @type {string} */
var t = Outer.Fn.Count;
Outer.Fn(1);
Outer.Fn("x");
Outer.Fn.Missing;
Outer.A = () => 1;
Outer.A.x = 1;
Outer.A.x.toFixed();
