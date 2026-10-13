namespace A {
    export namespace B {
        export const enum E {
            V1 = 1,
            V2 = A.B.E.V1 | 100,
        }
    }
}
namespace A {
    export namespace B {
        export const enum E {
            V3 = A.B.E["V2"] & 200,
            V4 = A.B.E[`V1`] << 1,
        }
    }
}
function partial(e: A.B.E): number {
    switch (e) {
        case A.B.E.V1: return 1;
        case A.B.E.V2: return 1;
        case A.B.E.V3: return 1;
    }
}
function complete(e: A.B.E): number {
    switch (e) {
        case A.B.E.V1: return 1;
        case A.B.E.V2: return 1;
        case A.B.E.V3: return 1;
        case A.B.E.V4: return 1;
    }
}
const fromLaterBlock: A.B.E = A.B.E.V4;
const zero: 0 = A.B.E.V3;

namespace P.Q { export enum F { a = 1 } }
namespace P.Q { export enum F { b = 2 } }
const dotted: P.Q.F = P.Q.F.b;

namespace Local { enum G { a = 1 } }
namespace Local { enum G { b = 2 } }
