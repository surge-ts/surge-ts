// @allowJs: true
// @checkJs: true
// @strict: false
// A function's `@type` is its full signature when one call signature of it
// takes the function's parameters; several do only once `noImplicitAny`
// intersects them (TS8030 without it), through a `@typedef` too.
// @filename: test.js
/** @typedef {{(s: string): 0 | 1; (b: boolean): 2 | 3 }} Gioconda */

/** @type {Gioconda} */
function monaLisa(sb) {
    return typeof sb === 'string' ? 1 : 2;
}

/** @type {{(s: string): 0 | 1; (b: boolean, c: number): 2 | 3 }} */
function oneFits(sb, c) {
    return 2;
}
