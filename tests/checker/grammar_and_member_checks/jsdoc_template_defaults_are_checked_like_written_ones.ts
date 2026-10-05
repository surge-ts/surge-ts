// @allowJs: true
// @checkJs: true
// @filename: file.js
/**
 * @template {string | number} [T=]
 * @typedef {[T]} D
 */

/**
 * @template {string | number} [T=string]
 * @template U
 * @typedef {[T, U]} E
 */

/**
 * @template [T=U]
 * @template [U=T]
 * @typedef {[T, U]} G
 */

/**
 * @template T
 * @template [U=T]
 * @param {T} a
 * @param {U} b
 */
function f1(a, b) {}

/**
 * @template {string | number} [T=string]
 * @template U
 * @param {T} a
 * @param {U} b
 */
function f2(a, b) {}
