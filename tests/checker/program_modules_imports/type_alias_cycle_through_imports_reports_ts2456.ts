// @filename: a.ts
import type { B } from './b';
export type A = B;
export type Fine = string;
// @filename: b.ts
import type { A, Fine } from './a';
export type B = A;
export type C = Fine;
// @filename: c.ts
import type { ns2 } from './d';
export namespace ns1 {
    export namespace nested {
        export type T = ns2.nested.T;
    }
}
// @filename: d.ts
import type { ns1 } from './c';
export namespace ns2 {
    export namespace nested {
        export type T = ns1.nested.T;
    }
}
