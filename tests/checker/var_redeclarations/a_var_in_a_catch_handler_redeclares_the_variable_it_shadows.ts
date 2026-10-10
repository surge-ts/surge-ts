// @strict: true
// A `var` in a `catch` handler naming the clause's binding is the function's
// variable of that name, which the binding only shadows inside the handler.
function f(x: boolean) {
    try { } catch (x) { var x: string; }
    try { } catch (x) { var x: boolean; }
}
function g() {
    try { } catch (z) { var z: string; }
    try { } catch (z) { var z: boolean; }
}
export {};
