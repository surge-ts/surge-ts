// @strictBindCallApply: true
class C {
    foo(this: this, a: number, b: string): string { return "" }
}
declare let c: C;
let f13 = c.foo.bind(c);
let f14 = c.foo.bind(undefined);
