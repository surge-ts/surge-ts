// @allowJs: true
// @checkJs: true
// @noImplicitAny: true
// @filename: a.ts
const o = { a: 1 };
const k = "b";
o[k] = 1;
const n = 5;
o[n] = 1;
const known = "a";
o[known] = 2;
class C { m = 1 }
const c = new C();
c[k] = 1;
function F() {}
const fk = "p";
F[fk] = 1;
const arr: number[] = [];
const len = "length";
arr[len] = 0;
const map: Record<string, number> = {};
map[k] = 1;
// @filename: b.js
const box = {};
const key = "x";
box[key] = 1;
const filled = { a: 1 };
filled[key] = 2;
