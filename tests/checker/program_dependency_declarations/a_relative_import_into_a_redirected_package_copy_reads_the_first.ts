// @filename: node_modules/a/index.d.ts
import { C } from "foo";
export const o: C;
// @filename: node_modules/a/node_modules/foo/index.d.ts
export class C {
    private x: number;
}
// @filename: node_modules/a/node_modules/foo/package.json
{ "name": "foo", "version": "1.2.3" }
// @filename: node_modules/foo/index.d.ts
export class C {
    private x: number;
}
// @filename: node_modules/foo/package.json
{ "name": "foo", "version": "1.2.3" }
// @filename: node_modules/foo/use.d.ts
import { C } from "./index";
export function use(o: C): void;
// @filename: index.ts
import { use } from "foo/use";
import { o } from "a";
use(o);
