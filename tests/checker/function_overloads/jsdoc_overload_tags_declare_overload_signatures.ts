// @allowJs: true
// @checkJs: true
// @strict: false
// @filename: overloads.js
/**
 * @overload
 * @param {number} a
 * @param {number} b
 * @returns {number}
 *
 * @overload
 * @param {string} a
 * @param {boolean} b
 * @returns {string}
 *
 * @param {string | number} a
 * @param {string | number} b
 * @returns {string | number}
 */
export function overloaded(a, b) {
    return a;
}
var o1 = overloaded(1, 2);
var o2 = overloaded("zero", "one");
var o3 = overloaded("a", false);

/**
 * @overload
 * @template T
 * @param {T[][]} array
 * @returns {T[]}
 */
/**
 * @param {unknown[]} array
 * @returns {unknown[]}
 */
function flat(array) {
    return array;
}
