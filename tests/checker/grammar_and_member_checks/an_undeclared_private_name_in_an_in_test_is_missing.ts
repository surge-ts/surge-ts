// @target: es2022
// `#x in o` whose name no enclosing class declares is missing from the right
// operand's type (`checkInExpression`).
class Foo {
    #field = 1;
    good(v: any) {
        return #field in v;
    }
    bad(v: any, f: Foo) {
        const a = #fiel in v;
        const b = #fiel in f;
        return a && b;
    }
}
