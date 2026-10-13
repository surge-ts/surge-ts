// @filename: a.ts
namespace values { export class C { } export var v = C; }
var gv = 1;
function gf() {}
declare module "m1" {
    import alias = values;
    export var a: typeof values;
    export var b: typeof gv;
    export var c: typeof gf;
    export var d: typeof alias;
    export var e: typeof missing;
}
// @filename: b.ts
import { a, b, c, d } from "m1";
