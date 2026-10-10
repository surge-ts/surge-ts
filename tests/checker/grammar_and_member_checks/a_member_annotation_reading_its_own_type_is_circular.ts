// @noImplicitAny: true
// A member annotated with `typeof this.<member>` reads its own type through
// itself (TS2502 at the member), directly or through another member; one that
// reads a member typed otherwise is fine.
declare class C {
    get foo(): typeof this.foo;
    a: typeof this.b;
    b: typeof this.a;
    c: typeof this.d;
    d: string;
}
