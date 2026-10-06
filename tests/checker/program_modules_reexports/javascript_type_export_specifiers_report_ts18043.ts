// @allowJs: true
// @checkJs: true
// @filename: index.js
/** @typedef {{ x: any }} JSDocType */
/** @callback Fn
 * @returns {void}
 */
/** @typedef {number} Both */
const Both = 1;
const value = 2;

export { JSDocType };
export { JSDocType as ThisIsFine };
export { Fn, Both, value };
