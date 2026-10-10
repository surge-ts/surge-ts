// @strict: true
// A class with no constructor of its own takes its base's, under its own type
// parameters: `new D(1, '')` infers `D<number, string>`, so the getter typed by
// the setter's `U` reads a `string`, which is not callable (TS6234).
class C<T, U> {
    x: T;
    get y() { return null as any; }
    set y(v: U) { }
    constructor(public a: T, private b: U) { this.x = a; }
}
class D<T, U> extends C<T, U> { }
var d = new D(1, '');
var r6 = d.y();
const n: number = d.a;
const s: number = d.y;
