// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: a.js
// A member declared by `this.x = v` in the constructor is typed from `v`,
// which may read the constructor's (JSDoc-typed) parameters; a generic
// class's member follows each instantiation.
class Bar {
    /** @param {number} x */
    constructor(x) {
        this.a = x;
    }
}
/** @template T */
class Foo {
    /** @param {T} x */
    constructor(x) {
        this.a = x;
        this.d = [x];
    }
}
/** @type {string} */
var s1 = new Bar(1).a;
var f = new Foo(1);
var g = new Foo(false);
/** @type {string} */
var s2 = f.d;
f.a = g.a;
