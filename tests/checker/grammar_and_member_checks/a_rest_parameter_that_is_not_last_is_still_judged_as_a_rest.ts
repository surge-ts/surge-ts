// @strict: true
// A rest parameter that is not last is parsed as an ordinary one (TS1014),
// and its annotation is still judged as a rest's: a type reference that is no
// array is TS2370, whatever kind of function declares it.
var f2 = (...x: Date, ...y: boolean) => { }
var f3 = (...x: string, ...y: boolean) => { }
function g1(...x: Date, ...y: boolean) { }
function g2(...x: Date, y: number) { }
var f4 = function (...x: Date, y: number) { }
class K { m(...x: Date, y: number) { } }
type T1 = (...x: Date, y: number) => void;
interface I1 { m(...x: Date, y: number): void }
function g3<T extends unknown[]>(...x: T, y: number) { }
function g4(...x: Array<number>, y: number) { }
