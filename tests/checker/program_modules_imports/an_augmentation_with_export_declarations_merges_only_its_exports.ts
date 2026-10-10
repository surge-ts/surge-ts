// An augmentation with an export declaration is no export context: its
// interface without `export` is local to the block and merges into nothing
// (TS2339 on both uses).
// @filename: f1.ts
export class A {}
// @filename: f2.ts
export class B {
    n!: number;
}
// @filename: f3.ts
import { A } from "./f1";
A.prototype.foo = function () { return undefined; }
declare module "./f1" {
    import { B } from "./f2";
    export { B } from "./f2";
    interface A {
        foo(): B;
    }
}
// @filename: f4.ts
import { A } from "./f1";
import "./f3";
declare let a: A;
let b = a.foo().n;
