// @strict: true
// Inside an instance member `this` is the class's polymorphic `this` type, a
// type parameter constrained to the instance: the instance itself does not
// suit it, `this` suits the instance, and a `this` written in the body names it.
class C1 {
    n = 1;
    f(x: number): this { return undefined; }
    g(): this { return new C1(); }
    h(): this { return this; }
    k() {
        let t: this = new C1();
        let u: this = this;
        let v: C1 = this;
        let w = <this>undefined;
        let x = undefined as this;
        const self = this;
        self.n = 2;
        this.n = 3;
        const arrow = () => this.h();
        return [t, u, v, w, x, arrow];
    }
    chain(): this {
        return this.h();
    }
    other(o: C1) {
        o.n = 4;
        const r: C1 = o.h();
        return r;
    }
    list() {
        var a: C1[];
        var a = [this, new C1()];
        var b: this[];
        var b = [this, null];
    }
}
class D1 extends C1 {
    d() {
        const c: C1 = this;
        const back: this = this.h();
        return [c, back];
    }
}
class G<T, U> {
    m() {
        var p = this;
        var p!: G<T, U>;
    }
}
