// @strict: false
namespace N2 {
    export class C { p: Q2; constructor() { this.p = new Q2(); } }
}
namespace N3 {
    export class C { p: Q3; m() { this.p = new Q3(); } }
}
namespace N4 {
    export class Ctx { decl: Decl4; }
    export function f(context: Ctx, ast: any) {
        context.decl = <Decl4>ast;
        context.decl = new Decl4();
    }
}
