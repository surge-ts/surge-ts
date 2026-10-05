// @filename: a.ts
export class A {}
export namespace N { export const v = 1; }
// @filename: b.ts
export type { N } from './a';
// @filename: c.ts
import type * as a from './a';
import { N } from './b';
import A = a.A;
import aa = a;
import V = N.v;
export { A, aa, V };
