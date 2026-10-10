// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: a.js
// A method of an object literal assigned to a function's expando is checked
// like any other, its JSDoc names included.
function N() {}
N.other = {
    /** @param {Missing} key */
    get(key) {}
};
/**
 * @template K
 */
function M() {}
M.prototype = {
    /** @param {K} key */
    get(key) {}
};
