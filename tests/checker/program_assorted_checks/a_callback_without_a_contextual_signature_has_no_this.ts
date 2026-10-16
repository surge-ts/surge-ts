// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: a.js
/**
 * @param {function(this: string, number): number} c
 */
function id1(c) { return c }
function id2(c) { return c }
id1(function () { return this.length; });
id2(function () { return this.length; });
// @filename: b.ts
declare function f(c: any): void;
f(function (n: number) { return this.length; });
declare function g(c: Function): void;
g(function () { return this.length; });
declare function h(...c: any[]): void;
h(function () { return this.length; });
declare function k(c: (this: string) => number): void;
k(function () { return this.length; });
f(() => 1);
