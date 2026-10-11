// @filename: a.ts
class A { a!: string }
export type { A as default };
export type { A as Named };

// @filename: b.ts
import A from './a';
import { Named } from './a';
export { A, Named };

// @filename: c.ts
import * as types from './b';
new types.A();
new types.Named();
export {};
