// @strict: true
enum E1 { X }
enum E2 { X }
declare function pick<T>(a: T, b: T): T;
pick(E1.X, E2.X);
declare function f1<T, U>(a: { w: (x: T) => U; r: () => T }, b: T): U;
f1({ w: (x) => x, r: () => E1.X }, E2.X);
enum E3 { A, B }
export const same: 0 = pick(E3.A, E3.B);
export const plain: string = pick(1, 2);
