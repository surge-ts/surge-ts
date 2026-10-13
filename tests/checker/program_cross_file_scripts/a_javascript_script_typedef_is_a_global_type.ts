// @allowJs: true
// @checkJs: true
// @filename: types.ts
export interface F {
    (): E;
}
export interface D<T extends F = F> {}
export const p: Plain = { a: "x" };
// @filename: other.js
/** @typedef {import("./types").D} E */
/** @typedef {{ a: number }} Plain */
// @filename: use.ts
const q: Plain = { a: 1 };
