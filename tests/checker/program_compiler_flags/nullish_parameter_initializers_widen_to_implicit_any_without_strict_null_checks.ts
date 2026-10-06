// @strict: false
// @noImplicitAny: true
// @allowJs: true
// @checkJs: true
function f(a = null, b = undefined, c = [], d = void 0, e = 1) { }
const g = (p = null) => p;
declare function take(callback: (value?: number) => void): void;
take((value = null) => {});
// @filename: b.js
function h(a = null, l = [], n = 1) { }
