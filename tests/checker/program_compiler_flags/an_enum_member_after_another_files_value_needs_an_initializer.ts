// @isolatedModules: true
// @target: esnext
// A member without an initializer is numbered from the one before it: after
// a string that is TS1061, and after a number another file holds TS18056
// under `isolatedModules`, which a single-file transpiler cannot see. A
// string built from another file's value is not syntactically one (TS18055).
// @filename: helpers.ts
export const n = 2;
export const s = "x";
// @filename: a.ts
import { n, s } from "./helpers";
enum A { a = n, b, c }
enum B { a = s, b }
enum D { a = n + 1, b }
enum E { a = `${s}`, b = s + "y", c = 1 + n, d = 2 + s }
