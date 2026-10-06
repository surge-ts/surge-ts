declare function f(s: string): string;
declare function f(n: number): number;
declare function f<T>(x: T): T;
const r1: number = f("");
const r2: string = f(true);
const r3 = f(true);
const r4: string = r3;
var v = {
    [f("")]: 0,
    [f(0)]: 0,
    [f(true)]: 0
};
