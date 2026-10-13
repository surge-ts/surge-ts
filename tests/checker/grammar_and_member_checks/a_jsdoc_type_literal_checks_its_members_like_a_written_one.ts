// @allowJs: true
// @checkJs: true
// @filename: a.js
/** @type {unique symbol} */
const foo = Symbol();
export { foo };
/** @typedef {{ [key: foo] boolean }} B */
/** @typedef {{ a: string b: number }} C */
/** @typedef {{ a: string, b: number }} D */
/** @typedef {{ [key: string]: number; c: number }} E */
