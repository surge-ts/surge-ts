// @strict: true
// @noImplicitAny: false
declare const o: { a: number; b: string; 1: boolean; [Symbol.iterator](): void };
declare const k: string;
declare const n: number;
const { [k]: x1, ...r1 } = o;
const t1: 7 = r1;
const { [n]: x2, ...r2 } = o;
const t2: 7 = r2;
const { ["a"]: x3, ...r3 } = o;
const t3: 7 = r3;
let r4: object, y4: unknown;
({ [k]: y4, ...r4 } = o);
