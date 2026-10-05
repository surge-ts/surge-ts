// @noImplicitAny: true
// @target: es2022
class A {
    y = this["z"];
    z = this["y"];
    w = this.v;
    v = 1;
}
