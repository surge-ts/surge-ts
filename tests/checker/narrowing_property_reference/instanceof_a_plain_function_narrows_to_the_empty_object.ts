// @strict: true
// @allowJs: true
// @checkJs: true
// @filename: a.ts
function F(this: any) { this.val = 1; }
declare var a: any;
if (a instanceof F) { a.val; }
declare var u: unknown;
if (u instanceof F) { u.val; }
declare var o: { x: number } | string;
if (o instanceof F) { o; }
declare const G: Function;
if (a instanceof G) { a.anything; }
declare const H: { (): void };
if (a instanceof H) { a.y; }
class C { c = 1; }
if (a instanceof C) { a.c; }
declare const K: new () => { k: number };
if (a instanceof K) { a.k; }
export {};
// @filename: b.js
/**
 * @constructor
 * @param {any} val
 */
function AtTop(val) { this.val = val }
/** @type {*} */
var v = 1;
if (v instanceof AtTop) {
    v.val
}
