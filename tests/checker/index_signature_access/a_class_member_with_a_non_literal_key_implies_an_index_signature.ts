// @strict: true
// A class member whose key is a `string` or `number` rather than a literal
// is no property but an index signature over every member it applies to
// (`getIndexInfosOfIndexSymbol`); an own written index signature wins.
declare var s: string;
declare var n: number;
class C {
    [s]: number = 1;
    [n]: string = "";
    x = true;
}
const c = new C();
const t1: "x" = c.foo;
const t2: "x" = c[3];
class F {
    [x: string]: number;
    [s]: string = "";
}
