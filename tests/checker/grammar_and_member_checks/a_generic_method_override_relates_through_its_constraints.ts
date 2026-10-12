class A { a!: string; }
class B { b!: string; }
interface I {
    f<T extends A>(): T;
}
class X implements I {
    f<T extends B>(): T { return undefined!; }
}
class Y implements I {
    f<T extends A>(): T { return undefined!; }
}
class Z implements I {
    f<T>(): T { return undefined!; }
}
class Base {
    g<T extends A>(x: T): T { return x; }
}
class Derived extends Base {
    g<T extends B>(x: T): T { return x; }
}
class Fine extends Base {
    g<T extends A>(x: T): T { return x; }
}
