// @target: es2015
// `getSignaturesOfSymbol` drops a body written right after another
// declaration of the same method, so the first duplicate alone types it.
class Foo { x!: number }
class Foo2 { x!: number; y!: number }
class C {
    [s: string]: () => Foo2;
    [""]() { return new Foo }
    [""]() { return new Foo2 }
}
class D {
    [s: string]: () => Foo2;
    m() { return new Foo2 }
    m() { return new Foo }
}
