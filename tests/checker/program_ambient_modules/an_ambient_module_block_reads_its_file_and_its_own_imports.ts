// A block's declarations see the enclosing file's declarations and imports,
// the block's own imports, and globals other files declare.
// @module: commonjs
// @filename: lib.d.ts
declare module "lib" {
    export class C { c: number; }
    export type Box<T> = { value: T };
}
declare module "uses-lib" {
    import { Box } from "lib";
    import lib = require("lib");
    var v: typeof lib;
    var w: lib.C;
    export type Boxed = Box<string>;
}
// @filename: f1.ts
export class A {}
// @filename: f2.ts
export class B {}
declare global {
    type GlobalHelper<T> = { helper: T };
}
// @filename: f3.ts
import { A } from "./f1";
import { B } from "./f2";
type Local = { local: true };
declare module "./f1" {
    interface A {
        foo(): B;
        bar: Local;
        baz: GlobalHelper<number>;
    }
}
