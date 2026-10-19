// @noImplicitAny: true
// A nested array pattern over a source that is not array-like reads each
// level off the iteration protocol (`checkIteratedTypeOrElementType`), and
// a union with a member no array covers is iterated too.
export {};
declare const pairs: Set<[number, string]>;
declare const gen: Generator<[number, [string, boolean]]>;
declare const values: number[] | Generator<number>;
declare const tuples: [number, string][];
declare const arr: number[];
const [, [n1, s1]] = pairs;
const t1: string = n1;
const t2: number = s1;
const [[m1, [x1, y1]], ...restGen] = gen;
const t3: string = m1;
const t4: number = x1;
const t5: string = y1;
const t6: string = restGen;
const [first, ...rest] = values;
const t7: string = first;
const t8: string = rest;
const [[a1, b1], [a2]] = tuples;
const t9: string = a1;
const t10: number = b1;
const [z1, z2] = arr;
const t11: string = z1;
const [[q1 = "d"]] = pairs;
const t12: boolean = q1;
