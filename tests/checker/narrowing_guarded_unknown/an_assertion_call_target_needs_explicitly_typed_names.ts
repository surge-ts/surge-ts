// @strict: true
// An assertion call needs an effects signature tsc reads through declared
// types alone: every name in its target declared with an explicit type
// (TS2775), and the target an identifier or qualified name (TS2776).
declare class Test { assert(value: unknown): asserts value; }
const annotated: (value: unknown) => asserts value = value => {};
const inferred = (value: unknown): asserts value => {};
function f(x: unknown, param: (value: unknown) => asserts value) {
    const assert = (value: unknown): asserts value => {};
    assert(typeof x === "string");
    const a = [assert];
    a[0](typeof x === "string");
    const t1 = new Test();
    t1.assert(typeof x === "string");
    const t2: Test = new Test();
    t2.assert(typeof x === "string");
    annotated(x);
    param(x);
    inferred(x);
}
class Prop {
    assert = (value: unknown): asserts value => {};
    typed: (value: unknown) => asserts value = value => {};
    other(x: unknown) {
        this.assert(x);
        this.typed(x);
    }
}
function g(items: Test[]) {
    for (const item of items) {
        item.assert(item);
    }
}
export {};
