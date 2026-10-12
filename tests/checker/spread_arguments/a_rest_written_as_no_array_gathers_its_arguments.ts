// @allowJs: true
// @checkJs: true
// @filename: a.ts
type G = (...args: string) => number;
declare const g: G;
g('a', 'b');
g('a');
// @filename: b.js
/**
 * @callback Foo
 * @param {...string} args
 * @returns {number}
 */

/** @type {Foo} */
export const x = () => 1
x('a', 'b')
