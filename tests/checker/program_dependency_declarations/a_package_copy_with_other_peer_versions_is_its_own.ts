// @filename: node_modules/d/index.d.ts
import P from "p";
export function d(p: P): void;
// @filename: node_modules/d/node_modules/p/index.d.ts
export default class P {
    private p: number;
}
// @filename: node_modules/d/node_modules/p/package.json
{ "name": "p", "version": "1.0.0", "peerDependencies": { "q": "*" } }
// @filename: node_modules/d/node_modules/q/package.json
{ "name": "q", "version": "1.0.0" }
// @filename: node_modules/e/index.d.ts
import P from "p";
export const e: P;
// @filename: node_modules/e/node_modules/p/index.d.ts
export default class P {
    private p: number;
}
// @filename: node_modules/e/node_modules/p/package.json
{ "name": "p", "version": "1.0.0", "peerDependencies": { "q": "*" } }
// @filename: node_modules/e/node_modules/q/package.json
{ "name": "q", "version": "2.0.0" }
// @filename: node_modules/f/index.d.ts
import P from "p";
export const f: P;
// @filename: node_modules/f/node_modules/p/index.d.ts
export default class P {
    private p: number;
}
// @filename: node_modules/f/node_modules/p/package.json
{ "name": "p", "version": "1.0.0", "peerDependencies": { "q": "*" } }
// @filename: node_modules/f/node_modules/q/package.json
{ "name": "q", "version": "1.0.0" }
// @filename: src/a.ts
import { d } from "d";
import { e } from "e";
import { f } from "f";
d(e);
d(f);
