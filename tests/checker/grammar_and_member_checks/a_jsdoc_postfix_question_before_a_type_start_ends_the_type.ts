// @allowJs: true
// @checkJs: true
// @strict: true
// A postfix `?` followed by a token that can start a type (`[`, or the `!`
// of a JSDoc non-null type) is a conditional type's `?`, so the type ends
// there and the braces expect their `}` (TS1005).
// @filename: prefixPostfix.js
/**
 * @param {number![]} x - number[]
 * @param {number?[]} a - parse error without parentheses
 * @param {...number?} f - number[] | null
 * @param {...number!?} g - number[] | null
 * @param {...number?!} h - parse error without parentheses (also nonsensical)
 * @param {...number?[]!} k - parse error without parentheses
 */
function f(x, a, f, g, h, k) {
}
