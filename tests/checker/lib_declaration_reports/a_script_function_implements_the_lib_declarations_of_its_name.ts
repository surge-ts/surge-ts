// @lib: es2023,dom
// A script function implements lib.dom's same-named `declare function`,
// which is checked as its overload where the lib declares it (TS2394).
function print(x: number) {}
function toString() {
    return "x";
}
function blur() {
    return 1;
}
