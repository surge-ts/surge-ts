// Two `export *` that export one name collide (TS2308); the first keeps the
// name with every meaning it has, and the second adds none of its own — the
// type `A` of `./a` is not exported beside `./e`'s value (TS2749).
// @filename: a.ts
export type A = number;
// @filename: e.ts
export const A = 1;
// @filename: f.ts
export * from "./e";
export type * from "./a";
// @filename: g.ts
import { A } from "./f";
A;
type _ = A;
