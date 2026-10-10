// @strict: true
// A namespace merged with a class is not inside the class: its private
// statics are as unreachable there as anywhere else (TS2341).
class C {
    private static sfn(id: string) { return 42; }
    private static sp = 1;
    static pub = 2;
}
namespace C {
    export function fn(): number {
        C.sp;
        C.pub;
        return C.sfn('a');
    }
    C.sfn('c');
}
C.sfn('b');

class G<T> {
    value!: T;
    private static make() { return 1; }
}
namespace G {
    export function fn<U>(x: U): number {
        return G.make();
    }
}

class K {
    private static s() { return 1; }
}
namespace N {
    export function f() {
        return K.s();
    }
}
