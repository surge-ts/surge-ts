// @allowJs: true
// @checkJs: true
// @filename: a.js
/**
 * @template const T
 * @typedef {[T]} X
 */

/**
 * @template const T
 */
class C { }

/**
 * @template private T
 * @param {T} x
 * @returns {T}
 */
function f(x) {
    return x;
}

/**
 * @template in T
 * @param {T} x
 */
function g(x) {}
