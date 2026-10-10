// @strict: true
// A `var` in a function body is the parameter of its name as well, so a
// different type is TS2403; a `catch` binding is a variable of its own clause.
function f(x: number) {
    var x: string;
}
function g(y: number) {
    var y: number;
}
function h(z = 1) {
    var z: string;
}
const arrow = (w: number) => {
    var w: string;
};
class K {
    m(v: number) {
        var v: string;
    }
}
function destructured([a = 1, b]: [number, string]) {
    var a: number;
    var b: string;
}
function args() {
    var arguments: IArguments;
}
try { } catch (e) {
    var e: string;
}
