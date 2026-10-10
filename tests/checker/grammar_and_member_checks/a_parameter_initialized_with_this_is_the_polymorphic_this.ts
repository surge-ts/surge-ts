// @strict: true
// A parameter initialized with `this` takes the polymorphic `this` type.
class C {
    n = 1;
    m(t = this) {
        const q: number = t;
        var t!: C;
    }
    k(t = this) {
        var t: this;
        return t.n;
    }
}
export {};
