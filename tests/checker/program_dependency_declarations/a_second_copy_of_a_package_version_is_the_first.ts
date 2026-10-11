// @filename: node_modules/a/index.d.ts
import X from "x";
export function a(x: X): void;
// @filename: node_modules/a/node_modules/x/index.d.ts
export default class X {
    private x: number;
}
export const one = 1 + 1;
// @filename: node_modules/a/node_modules/x/package.json
{ "name": "x", "version": "1.2.3" }
// @filename: node_modules/b/index.d.ts
import X from "x";
export const b: X;
// @filename: node_modules/b/node_modules/x/index.d.ts
content not parsed
// @filename: node_modules/b/node_modules/x/package.json
{ "name": "x", "version": "1.2.3" }
// @filename: node_modules/c/index.d.ts
import X from "x";
export const c: X;
// @filename: node_modules/c/node_modules/x/index.d.ts
export default class X {
    private x: number;
}
// @filename: node_modules/c/node_modules/x/package.json
{ "name": "x", "version": "1.2.4" }
// @filename: src/a.ts
import { a } from "a";
import { b } from "b";
import { c } from "c";
a(b);
a(c);
