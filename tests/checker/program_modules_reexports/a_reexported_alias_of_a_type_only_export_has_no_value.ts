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

// @filename: e.ts
function E() {}
export type { E };

// @filename: f.ts
import { E } from "./e";
namespace E {
    export const displayName = "E";
}
export { E };

// @filename: g.ts
import { E } from "./f";
E.displayName;

// @filename: h.ts
import { Named } from "./a";
namespace Named {}
export { Named };

// @filename: i.ts
import { Named } from "./h";
Named;
