// @target: esnext
// A private name stands only for a member's name, a property access's name or
// the left of an `in` test; anywhere else in a class it is TS1451.
class Foo {
    #field = 1;
    m(v: any) {
        const ok = #field in v && this.#field;
        const a = (#field) in v;
        const b = #field + 1;
        const c = [#field];
        const d = { [#field]: 1 };
        const e = v[#field];
        const f = !#field;
        const g = v in #field;
        return #field;
    }
}
