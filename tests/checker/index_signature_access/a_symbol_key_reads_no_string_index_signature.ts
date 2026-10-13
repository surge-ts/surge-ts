// @strict: true
// @target: es2015
const sym = Symbol()
type Both = { s: number, '0': number, [sym]: boolean } | { [n: number]: number, [s: string]: string | number }
declare var both: Both
let r = both[sym];
both[sym] = 'not ok'
type One = { [n: number]: number, [s: string]: string | number }
declare var one: One
let r2 = one[sym];
one[sym] = 'x';
declare const s: symbol;
let r3 = one[s];
type Keyed = { [sym]: boolean, [s: string]: string }
declare var keyed: Keyed
let r4 = keyed[sym];
