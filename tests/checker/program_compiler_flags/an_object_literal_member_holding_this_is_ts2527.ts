// @declaration: true
// Declaration emit cannot write a member whose inferred type is an object
// type holding the class's `this`: inside a type literal `this` names the
// literal (TS2527). An array or a function of `this`, a private or static
// member, and an annotated one are written as they are.
export class C {
    x1 = { a: this };
    x2 = [this];
    x3 = [{ a: this }];
    x4 = () => this;
    x5 = { f: () => this };
    private x6 = { a: this };
    static x7 = { a: this };
    x8: object = { a: this };
    f1() { return { a: this }; }
    f2() { return [this]; }
    get g() { return { a: this }; }
}
class Local {
    x1 = { a: this };
}
