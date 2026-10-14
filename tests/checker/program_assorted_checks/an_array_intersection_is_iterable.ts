// @strict: true
// @target: es2022
// An intersection with an array operand carries the array's iterator.
declare function pack<T>(...values: T[]): T[] & { n: number };
const packed = pack("a", "b");
export const spread: string[] = [...packed];
for (const value of packed) value.length;
export const count: number = packed.n;
declare const notIterable: number & { n: number };
export const bad = [...notIterable];
