var v1 = 1;
let l1 = "a";
function fn1() {}
type A1 = typeof v1;
type A2 = Array<typeof l1>;
type W<T> = { v: T };
interface I { p: W<typeof fn1> }
declare const a1: A1;
declare const a2: A2;
declare const a3: I;
const n: string = a1;
const s: number[] = a2;
a3.p.v();
