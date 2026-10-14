// @strict: true
// An array literal is a tuple only under a tuple-like contextual type: a type
// parameter whose constraint also admits a string infers an array, while the
// `[]` of `readonly unknown[] | []` makes it a tuple.
declare function either<T extends string | number[]>(x: T): T;
export const fourth: number = either([1, 2])[3];
declare function tupled<T extends readonly unknown[] | []>(x: T): T;
const pair = tupled([1, "a"]);
export const second: string = pair[1];
export const beyond = pair[2];
declare function concat<T extends unknown[], U extends unknown[]>(a: [...T], b: [...U]): [...T, ...U];
export const wrong: [number] = concat([1, 2], ["x"]);
