// @module: commonjs
// @esModuleInterop: true
// A declaration file, and a `declare module` block, is an export context —
// every declaration exported — until it has an export declaration of its
// own. `export import` and `export default interface` are declarations with
// an `export` modifier, not export declarations.
// @filename: lib1.d.ts
declare function hidden(): void;
export declare function shown(): void;
// @filename: lib2.d.ts
declare function hidden2(): void;
export declare function shown2(): void;
export {};
// @filename: lib3.d.ts
export import l1 = require("./lib1");
declare function hidden3(): void;
// @filename: lib4.d.ts
export default interface Face { x: number }
declare function hidden4(): void;
// @filename: decl.d.ts
declare module "m1" {
    function hiddenA(): void;
}
declare module "m2" {
    function hiddenB(): void;
    export function shownB(): void;
    export {};
}
declare module "m3" {
    function hiddenC(): void;
    namespace hiddenC { const z: number; function inner(): void; }
    interface Opts { a: number }
    export = hiddenC;
}
// @filename: use.ts
import * as l1 from "./lib1";
import * as l2 from "./lib2";
import * as l3 from "./lib3";
import * as l4 from "./lib4";
import * as m1 from "m1";
import * as m2 from "m2";
import { inner } from "m3";
import { Opts } from "m3";
import m3 = require("m3");
l1.hidden();
l1.shown();
l2.hidden2();
l2.shown2();
l3.hidden3();
l3.l1.shown();
l4.hidden4();
m1.hiddenA();
m2.hiddenB();
m2.shownB();
m3();
m3.z;
inner();
let o: Opts;
