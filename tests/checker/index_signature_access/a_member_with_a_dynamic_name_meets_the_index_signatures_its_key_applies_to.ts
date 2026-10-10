// @strict: true
// A member whose computed key names no property is still checked against
// the index signatures its key's type applies to, and for initialization.
class Foo { x!: number }
class Foo2 { x!: number; y!: number }
class C {
    [s: string]: Foo2;
    get [1 << 6]() { return new Foo }
    set [1 << 6](p: Foo2) { }
}
class D {
    [s: number]: Foo2;
    get [1 << 6]() { return new Foo }
}
declare var s: string;
class E {
    [s]: number = 0;
    [+s]: typeof s;
    [Symbol()]: number;
}
