// Two stars reaching the same module export the same symbols: no collision,
// and the plain star keeps them values (`export type *` alone would not).
// @filename: a.ts
export class A {}
export class B {}
// @filename: d.ts
export type * from "./a";
export * from "./a";
// @filename: e.ts
import { A, B } from "./d";
let _: A = new A();
let __: B = new B();
