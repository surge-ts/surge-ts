class C { foo = ""; }
namespace M {
    var C = 1;
    class D extends C { }
}
namespace M2 {
    var Q = 1;
    class D extends Q { }
}
function f() {
    var C = "s";
    class E extends C {}
}
interface I {}
declare const I: { new (): I };
class J extends I {}
