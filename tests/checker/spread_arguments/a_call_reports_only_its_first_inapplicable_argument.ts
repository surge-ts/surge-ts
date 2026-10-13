// @strict: true
declare function h(a: { a: number, b: string }, b: { c: boolean }): void;
h({ a: 1 }, undefined);
declare function k<T extends { a: number, b: string }, V extends { c: boolean }>(a: T, b: V): void;
k({ a: 1 }, undefined);
declare function m(a: string, b: number): void;
m(1, "x");
declare function n(a: { a: number }[], b: string): void;
n([{ a: "x" }], 1);
