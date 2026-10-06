// @allowJs: true
// @checkJs: true
// @filename: a.js
/**
 * @param {object} obj - takes the destructured parameter's place
 * @param {string} obj.a
 * @param {string} obj.b
 * @param {string} x
 */
function bad1(x, {a, b}) {}

/**
 * @param {string} y
 * @param {{a: string, b: string}} obj
 */
function bad2(x, {a, b}) {}

/**
 * @param {object} unmatched
 */
function bad3(x) {}
