// @allowJs: true
// @checkJs: true
// @strict: true
// `@template in out T` names `T`: the modifiers come first.
// @filename: a.js
/**
 * @template out T
 * @typedef {Object} Covariant
 * @property {T} x
 */
/** @type {Covariant<unknown>} */
let superCovariant = { x: 1 };
/** @type {Covariant<string>} */
let subCovariant = { x: "" };
superCovariant = subCovariant;
subCovariant = superCovariant;
