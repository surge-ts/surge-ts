// @module: commonjs
// `import A = b.a.A` passes through `b.a`, which `b` re-exports from an
// `import type`: the alias references a type-only declaration (TS1380). `b.A`
// re-exports an import alias, which is no type-only declaration of its own.
// @filename: a.ts
export class A {}
// @filename: b.ts
import type * as a from './a';
import A = a.A;
import aa = a;

const x = 0;
export { a, A, x };
// @filename: c.ts
import * as b from './b';
import A = b.a.A;
import AA = b.A;

import x = b.x;
console.log(x);
