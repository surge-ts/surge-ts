// @strict: true
// `find` with a predicate callback takes the lib's narrowing overload
// (`find<S extends T>(predicate: (value: T, …) => value is S): S | undefined`),
// whether the predicate is written or inferred from the callback's body.
declare const ns: number[];
const isTwo = (v: number) => v === 2;
export const inferredInline: 2 | undefined = ns.find((v) => v === 2);
export const inferredNamed: 2 | undefined = ns.find(isTwo);
export const written: 2 | undefined = ns.find((v): v is 2 => v === 2);
export const plain: 2 | undefined = ns.find((v) => v > 2);
