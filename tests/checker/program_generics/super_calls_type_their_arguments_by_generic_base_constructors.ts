// @strict: true
class A<T1, T2> {
    constructor(private map: (value: T1) => T2) {}
}
class C extends A<number, string> {
    constructor() { super(value => value.foo); }
}
class B { constructor(f: (v: number) => void) {} }
class D extends B { constructor() { super(v => v.foo); } }
