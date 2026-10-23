// @surge-compare: messages
// @target: es2022
function arrayKeeps<T>(x: T & number[]): T { return x; }
function arrayIsArray<T>(x: T & number[]): number[] { return x; }
function arrayLength<T>(x: T & number[]): number { return x.length + x[0]; }
function arrayIterates<T>(x: T & number[]): number { let s = 0; for (const n of x) { s += n; } return s; }
function arraySpreads<T>(x: T & number[]): number[] { return [...x]; }
function arrayMethod<T>(x: T & string[]): string { return x.map((s) => s.trim()).join(","); }
function arrayKeys<T>(x: T & number[]): keyof (T & number[]) { return "length"; }
function tupleKeeps<T>(x: T & [string, number]): T { return x; }
function tupleElements<T>(x: T & [string, number]): number { return x[0].length + x[1]; }
function signatureKeeps<T>(x: T & (() => void)): T { x(); return x; }
function signatureCalls<T>(x: T & ((n: number) => string)): string { return x(1); }
function signatureIsSignature<T>(x: T & ((n: number) => string)): (n: number) => string { return x; }
function unionMember<T>(x: T & (number[] | string)): T { return x; }

type Listed<T> = T & string[];
type Named<T> = Listed<T> & { name: string };
function nested<T>(x: Named<T>): string { return x.name + x.join(""); }

function arrayMissing<T>(x: T & number[]) { return x.missing; }
function tupleNotAssignable<T>(x: T): T & [string, number] { return x; }
function arrayNotAssignable<T>(x: T): T & number[] { return x; }
function signatureWrongArgument<T>(x: T & ((n: number) => string)): string { return x("a"); }
