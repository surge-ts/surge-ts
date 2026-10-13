// @allowJs: true
// @checkJs: true
// @strict: true
// @module: commonjs
// @filename: mod1.js
exports.K = class K {
    values() { return K; }
};
exports.B = class {
    constructor() { this.x = 1; }
};
Object.defineProperty(exports, "rw", { get() { return 98122 }, set(_) { } });
Object.defineProperty(exports, "rv", { value: 1, set(_) { } });
// @filename: main.js
const { K, B, rw } = require("./mod1");
/** @param {K} k @param {B} b */
function f(k, b) {
    k.values();
    b.x;
    k.nope;
}
/** @type {string} */
const s = rw;
