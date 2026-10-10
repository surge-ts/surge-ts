// @strict: true
// @lib: es2023
const upper = Array.from("abc").map((c) => c.toUpperCase());
const wrongUpper: number[] = upper;
const codes = Array.from("abc", (c) => c.charCodeAt(0));
const wrongCodes: string[] = codes;
declare function firstOf<T>(source: ArrayLike<T>): T;
const first: number = firstOf("abc");
declare function lastOf<T>(source: Iterable<T>): T;
const last: number = lastOf("abc");
