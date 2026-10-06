namespace Foo.Bar {
    class Greeter { }
    export class Pub { }
    function foo(): Foo.Bar.Greeter { return new Greeter(); }
    var a: Bar.Greeter;
    var b: Foo.Bar.Pub;
    interface I {}
    var c: Foo.Bar.I;
    type T = number;
    var d: Bar.T;
    var e: Greeter;
}
namespace N {
    namespace Inner { export class X {} }
    var e: N.Inner.X;
    var f: Inner.X;
}
namespace Foo {
    var g: Foo.Bar.Pub;
}
namespace X.A.C {
    export interface Z { }
}
namespace X.A.B.C {
    export class W implements A.C.Z { }
}
namespace X.A.B.C {
    namespace A { }
}
