// @strict: true
// A computed name reading a `unique symbol` const binds like a literal, so an
// overload with no implementation under it is TS2391; a plain `symbol` binds
// nothing and is left alone.
declare const u1: unique symbol;
declare const u2: unique symbol;
const u3 = Symbol();
const u4: symbol = Symbol();
let u5 = Symbol();
class C1 {
    [u2](): void;
    [u1](): void;
    [u1]() { }
}
class C2 {
    [u3](): void;
}
class C3 {
    [u4](): void;
}
class C4 {
    [u5](): void;
}
class C5 {
    [u1](): void;
    [u1](x?: number): void;
    [u1](x?: number) { }
}
class C6 {
    [u1](): void;
    u1() { }
}
export {};
