// @strict: true
// An enum object's reverse mapping is a readonly `number` index signature of
// `string` values. `--` or `++` through it reads a `string` (TS2356) and
// writes a readonly index (TS2542), whatever the key: a `number`, a literal,
// or an unresolved name, which is tsc's error type and lands on the numeric
// index signature first.
enum ENUM1 { A, B }
declare const n: number;
ENUM1[n]--;
ENUM1[0]--;
const r: number = ENUM1[n];
ENUM1[A]--;
const q: number = ENUM1[A];

declare const record: { readonly [key: string]: number };
declare const key: string;
record[key]++;
