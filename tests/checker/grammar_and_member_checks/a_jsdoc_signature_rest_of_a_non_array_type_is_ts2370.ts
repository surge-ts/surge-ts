// @allowJs: true
// @checkJs: true
// @strict: true
// tsgo reparses a `@callback` or `@overload` `{...T}` parameter as a rest of
// type `T` itself, so a `T` that is no array is TS2370 at the tag.
// @filename: a.js
/**
 * @callback Foo
 * @param {...string} args
 * @returns {number}
 */
/**
 * @callback Arr
 * @param {...Array<number>} nums
 * @returns {void}
 */
/**
 * @overload
 * @param {...number} args
 * @returns {number}
 */
/**
 * @param {...any} args
 * @returns {number}
 */
function h(...args) { return 1; }
