// @module: commonjs
// @target: es2015
namespace x {
    interface c {
    }
    export interface d { v: number }
}
export public import a = x.c;
export private import b = x.c;
export static import c = x.c;
var b: a;
import e = x;
var f: e.c;
var g: e.d;
