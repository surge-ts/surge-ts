namespace A.B {
    export var x: number;
}
namespace A {
    namespace B {
        export var x: string;
    }
}
var x: number;
var x = A.B.x;
