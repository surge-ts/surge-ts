// A `paths` target naming a directory loads it as tsc's
// `loadNodeModuleFromDirectory` does: its package.json entry before its index.
// @paths: {"pkg": ["./packages/pkg"]}
// @filename: packages/pkg/package.json
{ "name": "pkg", "types": "./dist/index.d.ts" }
// @filename: packages/pkg/dist/index.d.ts
export declare const fromPkg: number;
// @filename: packages/pkg/index.d.ts
export declare const fromIndex: number;
// @filename: src/index.ts
import { fromPkg } from "pkg";
const value: string = fromPkg;
