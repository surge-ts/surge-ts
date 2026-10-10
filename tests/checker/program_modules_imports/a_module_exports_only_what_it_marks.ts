// A module exports the declarations it marks `export`; the rest stay local
// even through a namespace import (TS2694, TS2339).
// @filename: a.ts
interface A { x: number }
function f() {}
export const y = 1;
// @filename: b.ts
import * as ns from "./a";
let v: ns.A;
ns.f();
ns.y;
