// @strict: true
declare function f<T>(g: () => T): T;
const x = f(() => x);
declare function v(g: () => void): number;
const y = v(() => y);
declare function o<T>(c: { fields: () => T }): T;
const z = o({ fields: () => ({ a: z }) });
declare function p<T>(c: { fields: T | (() => T) }): T;
const w = p({ fields: () => ({ a: w }) });
const u = o({ fields() { return { a: u }; } });
const s = o({ fields: () => () => s });
