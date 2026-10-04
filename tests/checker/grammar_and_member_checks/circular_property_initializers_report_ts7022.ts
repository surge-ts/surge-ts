// @noImplicitAny: true
// @target: es2022
class A {
    foo = this.bar;
    bar = this.foo;
    baz = this.baz;
    qux = this.foo;
    quux = 1;
    later = this.quux;
    #p = this.#q;
    #q = this.#p;
    static s1 = A.s2;
    static s2 = A.s1;
}
