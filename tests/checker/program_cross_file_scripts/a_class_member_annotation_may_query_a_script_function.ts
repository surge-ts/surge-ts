// @filename: a.ts
function F(n: number) { return n; }
// @filename: b.ts
class C {
    static s(o: typeof F) { return o(1); }
    m(o: typeof F) {}
}
function G() {}
class D {
    m(o: typeof G) {}
}
new C().m(F);
