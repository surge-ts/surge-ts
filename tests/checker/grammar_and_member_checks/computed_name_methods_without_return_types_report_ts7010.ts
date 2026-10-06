// @strict: true
const lit = "x";
declare const sym: unique symbol;
let notLit: string = "y";
interface I {
    [lit]();
    [sym]();
    [notLit]();
}
declare class C {
    [lit]();
    [notLit]();
}
