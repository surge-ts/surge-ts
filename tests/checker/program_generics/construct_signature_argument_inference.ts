// A generic constructor handed to a `new (a: T) => U` parameter is
// instantiated in that signature's context and infers `U` from its return.
interface I { new <T>(x: T): T; }
declare var i: I;
declare var a: { new <T>(x: T): T; };

function foo3<T, U>(x: T, cb: new (a: T) => U, y: U) {
    return new cb(x);
}
var r7 = foo3(null, i, '');
var r7b = foo3(null, a, '');

function k<T, U>(x: T, cb: new (a: T) => U, y: U): [T, U] { return null as any; }
var r9: [number, number] = k(null, i, '');

interface Box<T> { value: T; }
declare var boxer: new (value: number) => Box<number>;
function make<V>(ctor: new (value: number) => Box<V>): V { return null as any; }
const made: string = make(boxer);
