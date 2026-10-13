// @strict: true
// @target: es2015
const sym = Symbol()
type Both = { s: number, '0': number, [sym]: boolean } | { [n: number]: number, [s: string]: string | number }
declare var both: Both
let r = both[sym];
both[sym] = 'not ok'
type Keyed = { [sym]: boolean, [s: string]: string }
declare var keyed: Keyed
let r4 = keyed[sym];
const other = Symbol.for("other");
declare const record: Record<symbol, string>;
let r5 = record[other];
