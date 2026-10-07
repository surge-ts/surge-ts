// @strict: true
function f1(x: unknown) {
    if (x && "a" in x) {
        x.a;
    }
}
function f2(x: {}) {
    if ("a" in x) { }
}
function f3(x: object) {
    if ("a" in x) { }
}
