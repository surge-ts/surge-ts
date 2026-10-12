// @strict: true
// @allowJs: true
// @checkJs: true
// @filename: a.ts
declare const k: string;
declare const n: number;
declare const lit: "Math" | "JSON";
this[k];
this[k] = 0;
globalThis[k];
globalThis[n] = 1;
this["a" + "b"] = 0;
globalThis[lit];
const g = globalThis;
g[k];
// @filename: b.js
this["a" + "b"] = 0;
