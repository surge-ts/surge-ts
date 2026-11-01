// @surge-compare: messages
// @strict: true
// @target: es2015
let a = [{ a: 1, b: 2 }, { a: "abc" }, {}][0];
a = 0;
let y = [{}, { p: 1, q: "s" }, { q: true, p: "t" }][0];
y = 0;
let n = [{ p: { x: 1 } }, { p: { y: 1, x: 2 } }, { p: { z: 1 } }][0];
n = 0;
declare const c1: boolean, c2: boolean;
let t = c1 ? { c: 1 } : c2 ? { e: 1, d: 2 } : { d: 3, c: 4 };
t = 0;
declare function f<T>(...args: T[]): T;
let g = f({}, { g: 1 }, { h: 2, g: 3 });
g = 0;
