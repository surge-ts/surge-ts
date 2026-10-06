// @module: commonjs
// @filename: value.ts
var x = 1;
export = x;
// @filename: klass.ts
class K { a = 1; }
export = K;
// @filename: merged.ts
function f() {}
namespace f { export interface A { a: number } }
export = f;
// @filename: use.ts
import x = require("./value");
import K = require("./klass");
import f = require("./merged");
let a: x.A;
let b: K.A;
let c: f.A;
let d: K.a;
