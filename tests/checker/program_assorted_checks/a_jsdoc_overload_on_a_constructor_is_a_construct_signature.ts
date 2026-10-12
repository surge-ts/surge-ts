// @strict: true
// @allowJs: true
// @checkJs: true
// @target: esnext
// @filename: a.js
class C {
    /**
     * @overload
     * @param {string} a
     */
    /**
     * @overload
     * @param {number} a
     * @param {number} b
     */
    /**
     * @param {string | number} a
     * @param {number} [b]
     */
    constructor(a, b) {}
}
new C("x");
new C(1, 2);
new C(1);
