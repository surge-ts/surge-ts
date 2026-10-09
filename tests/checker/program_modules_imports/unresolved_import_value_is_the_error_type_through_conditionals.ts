// @strict: true
// @noPropertyAccessFromIndexSignature: true
import { x } from "nope";
type C1 = typeof x extends string ? 1 : 2;
let c1: C1 = 3;
type D<T> = T extends string ? 1 : 2;
let d1: D<typeof x> = 3;
const o = { p: x };
type E = typeof o["p"] extends string ? 1 : 2;
let e1: E = 3;
type Out<T> = { [k in keyof T as T[k] extends string ? never : k]: T[k] };
declare function out<T>(t: T): Out<T>;
const remapped = out({ p: x });
remapped.p;
x<Missing>();
new x((c) => c);
