// @strict: false
declare const u: readonly [1, 2] | undefined;
const e = u || [9, 0] as const;
const f: 7 = e;
declare const o: { a: number };
const g = o || "x";
const h: 7 = g;
const k = "a" || 1;
const l: 7 = k;
