// @allowJs: true
// @checkJs: true
// @filename: a.js
var wat = require('./a');
module.exports = class C {};
module.exports.D = class D {};
wat.E;
// @filename: b.js
var M = class {};
M.z = 1;
M.z;
var Outer = {};
Outer.Inner = class {};
Outer.Inner.Message = function () {};
function mixin() {
    return class {};
}
mixin().nope;
