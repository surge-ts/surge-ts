namespace A {
    export var Point = { x: 0, y: 0 };
}
namespace B {
    var A = { x: 0, y: 0 };
    import Point = A;
}
namespace X {
    export class Y { name: string = ""; }
}
namespace Z {
    import Y = X.Y;
    var Y = 12;
}
namespace q {
    export const Q = {};
}
namespace r {
    export import Q = q.Q;
    export type Q = number;
}
namespace s {
    import Q = r.Q;
    const Q = 0;
}
