// @allowJs: true
// @checkJs: true
// @allowUnreachableCode: false
// A JavaScript function's `@returns {never}` is its return annotation: what
// follows a call to it is unreachable (TS7027).
// @filename: a.js
/** @returns {never} */
function failJs() { throw new Error(); }
/** @param {number} x */
function g(x) {
    if (!!true) {
        failJs();
        x;
    }
}
