// @allowJs: true
// @checkJs: true
// @module: commonjs
// @target: es2022
// @filename: a.js
exports.b.c = 1;
exports.b = { c: 2 };
{
    exports.d;
}
exports.d = 1;
if (exports.e) { }
exports.e = 1;
module.exports.jj = module.exports.j;
module.exports.j = function j() {};
module.exports.j;
function f() { return exports.k; }
const g = () => exports.k;
exports.k = 1;
Object.defineProperty(exports, "h", { value: 1 });
exports.h;
exports.m;
exports["m"] = 1;
class K {
    p = exports.q;
}
exports.q = 1;
