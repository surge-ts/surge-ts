// @strict: true
// @allowJs: true
// @checkJs: true
// @target: esnext
// @filename: b.js
class E {
    /**
     * @overload
     * @param {string} a
     * @param {number} b
     */
    /**
     * @overload
     * @param {number} a
     */
    /**
     * @param {number | string} a
     */
    constructor(a, b) {}
}
