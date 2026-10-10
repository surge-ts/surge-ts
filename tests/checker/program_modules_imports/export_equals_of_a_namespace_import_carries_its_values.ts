// `export = ns` of an import binding is an alias of what the import binds:
// an importer reads the namespace's values, and a type-only export is none.
// @module: commonjs
// @esModuleInterop: true
// @filename: a.ts
class A {}
export type { A };
export class V {}
// @filename: b.ts
import * as a from "./a";
export = a;
// @filename: d.ts
import b = require("./b");
new b.A();
new b.V();
const value: number = b;
