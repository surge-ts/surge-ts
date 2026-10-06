// @allowJs: true
// @checkJs: true
// @noImplicitAny: true
// @filename: mod.js
exports.n = {};
exports.n.K = function () {
    this.x = 10;
};
exports.fn2 = Math.min;
exports.fn1();
module.exports.b = function b() {};
module.exports.b.cat = "cat";
function f() {
    exports.nested = 1;
}
if (Math.random()) {
    exports.inBlock = true;
}
exports.nested;
// @filename: reassigned.js
function a() {}
exports.apply = a;
exports.apply = "ok";
exports.apply.toUpperCase();
// @filename: use.js
const mod = require("./mod");
mod.nested;
mod.inBlock;
mod.fn2(1, 2);
mod.missing;
