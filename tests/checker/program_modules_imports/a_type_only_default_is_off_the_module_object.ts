// @module: commonjs
// @esModuleInterop: true
// A default export whose alias chain passes through `import type` is a type
// only, which keeps it off the module's namespace object as `export type`
// keeps a named export off it (TS2339).
// @filename: a.ts
export class A {}
// @filename: b.ts
import type * as types from './a';
export default types;
// @filename: f.ts
import * as types from './b';
new types.default.A();
