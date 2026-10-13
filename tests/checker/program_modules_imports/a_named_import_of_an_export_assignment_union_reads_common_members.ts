// @filename: foo.ts
declare let x: { a: string; c: string; } | { b: number; c: number; };
export = x
// @filename: bar.ts
import { a, b, c, d } from './foo';
const n: boolean = c;
