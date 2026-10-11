// @strict: false
// Namespaces of one name merge their exported members across blocks, so a
// dotted `namespace A.B.C` written before `namespace A { export namespace B {
// export class C } }` is a namespace placed before the class it merges with
// (TS2434), though no single block holds both.
namespace A.B.C {
    export class Point {
        x: number;
    }
}

namespace A {
    export namespace B {
        export class C {
            name: string;
        }
    }
}

// Unexported members stay local to their block and merge with nothing.
namespace D {
    namespace E { export const v = 1; }
}
namespace D {
    class E {}
}
