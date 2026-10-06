class A {
    static s = "k";
    i = "j";
    static [this.c] = 1;
    [this.d] = 2;
    [this.i]() {}
    get [this.q]() { return 1; }
}

class C {
    static readonly c: "foo" = "foo";
    static bar = class Inner {
        static [this.c] = 123;
        [this.c] = 123;
    };
}
