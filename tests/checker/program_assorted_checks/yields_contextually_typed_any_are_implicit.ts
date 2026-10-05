// @noImplicitAny: true
// @target: es2015
declare function f(x: any): void;
declare function g(x: number): void;
function* g5() {
    f(yield);
    g(yield);
    const a: any = yield;
}
