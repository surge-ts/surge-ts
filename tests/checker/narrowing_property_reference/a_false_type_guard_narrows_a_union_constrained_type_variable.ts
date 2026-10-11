type Box<T> = { item: T };
declare function isBox(x: any): x is Box<any>;
declare function isUndefined(x: unknown): x is undefined;
declare function unbox<T>(x: Box<T>): T;
function g1<T extends Box<T> | undefined>(x: T) {
    if (!isUndefined(x)) {
        unbox(x);
    }
}
function g2<T extends Box<T> | undefined>(x: T) {
    if (!isBox(x)) {
        unbox(x);
    }
}
function g3<T extends Box<T> | undefined>(x: T) {
    if (isBox(x)) {
        unbox(x);
    }
}
type A = { kind: 'a'; a: number };
type B = { kind: 'b'; b: string };
declare function isA(x: A | B): x is A;
function g4<T extends A | B>(x: T) {
    if (!isA(x)) {
        x.b.toUpperCase();
    } else {
        x.a.toFixed();
    }
}
export {};
