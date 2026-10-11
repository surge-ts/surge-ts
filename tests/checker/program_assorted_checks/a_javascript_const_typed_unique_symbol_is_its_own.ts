// @allowJs: true
// @checkJs: true
// @target: es2015
// @filename: index.js
/** @type {unique symbol} */
const x = Symbol()
/** @type {unique symbol} */
const y = Symbol()

/** @type {typeof x} */
let z = x

z == y
