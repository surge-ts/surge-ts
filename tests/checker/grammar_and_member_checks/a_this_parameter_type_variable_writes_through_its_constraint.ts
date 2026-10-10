// @strict: true
// A `this: T` receiver is judged by `T`'s constraint: a function outside the
// class cannot write its private member (TS2341).
class MyClass {
    private p: number = 123;
}
function e1<T extends MyClass>(this: T, p: number) {
    this.p = p;
}
function e2<T extends MyClass>(this: T, p: number) {
    const v = this.p;
}
const e4 = function <T extends MyClass>(this: T, p: number) {
    this.p = p;
};
