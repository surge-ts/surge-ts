// @strict: true
// @lib: es2023
declare const nested: number[][];
const once = nested.flat();
const wrongOnce: string[] = once;
const literal = [[1], [2, 3]].flat();
const wrongLiteral: string[] = literal;
declare const deep: (string | string[][])[];
const twice = deep.flat(2);
const wrongTwice: number[] = twice;
const kept = deep.flat(0);
const wrongKept: number = kept;
declare const pairs: [string, number][];
const spread = pairs.flat();
const wrongSpread: boolean[] = spread;
