class C { static foo() { } static make() { return new C(); } }
const a: number = C.foo();
const b: number = C.make();
var r = 1 + C.foo();
