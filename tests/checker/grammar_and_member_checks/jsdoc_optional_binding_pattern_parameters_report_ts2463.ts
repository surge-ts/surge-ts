// @allowJs: true
// @checkJs: true
// @filename: a.js
/**
 * @typedef Foo
 * @property {string} a
 */

/**
 * @param {Foo} [options]
 */
function f({ a = "a" }) {}

/**
 * @param {Foo} options
 */
function h({ a }) {}

/** @param {Foo} [named] */
function i(named) {}
