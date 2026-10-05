namespace N {
    class Base { public foo: string = ""; }
    interface I { foo: string; }
    type T = string;
    export interface Shared { s: number; }
    declare var b1: Base;
    const t1: T = "ok";
}
namespace N {
    class Base { private foo: string = ""; }
    interface I { bar: number; }
    type T = number;
    declare var a: { foo: string; };
    declare var b: Base;
    declare var i: I;
    declare var s: Shared;
    a = b;
    const x: string = i.bar;
    const t: T = "s";
    const y: string = s.s;
}
