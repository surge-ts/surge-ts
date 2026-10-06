// @allowJs: true
// @checkJs: true
// @filename: a.js
/**
 * @param {number} a
 * @param {number} [b]
 * @param {number} c
 */
function foo(a, b, c) {}

/**
 * @param {string=} a
 * @param {?number} b
 */
function bar(a, b) {}

/**
 * @param {number} [a]
 * @param {number} b
 */
function baz(a, b = 1) {}

/**
 * @param {number} [a]
 * @param {...*=} rest
 */
function qux(a, ...rest) {}

/** @param {...*=} args */
function quux(...args) {}
