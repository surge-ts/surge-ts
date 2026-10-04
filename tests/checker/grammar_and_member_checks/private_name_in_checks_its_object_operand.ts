// @target: es2022
class Foo {
    #field = 1;
    check(v: any, n: null, u: unknown) {
        const a = #field in (v as unknown);
        const b = #field in n;
        for (let d in #field in v) { }
        if (#field in v) { v; }
        return #field in u;
    }
}
