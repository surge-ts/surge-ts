// @strict: true
// A property initialized with the polymorphic `this` — `this` itself, a read
// of such a member, or a call of a method returning `this`, inherited ones
// included — has the `this` type, which a write through `this` targets as
// the body's own `this`: an instance of the class or a subclass is not
// assignable to it (TS2322).
class C {
    self = this;
    c = new C();
    foo() { return this; }
    f1() {
        this.self = this.c;
        this.self = this;
        this.self = this.foo();
        const n: number = this.self;
    }
}
class D extends C {
    self1 = this;
    self2 = this.self;
    self3 = this.foo();
    d = new D();
    bar() {
        this.self = this.self1;
        this.self = this.self2;
        this.self = this.self3;
        this.self1 = this.self;
        this.d = this.self;
        this.self = this.d;
        this.c = this.d;
    }
}
const outside = new C();
outside.self = new C();
