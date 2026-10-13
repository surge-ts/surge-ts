// `export =` of a declared value typed `typeof import(...)`: importers read
// the target module's exports through it, whichever import form they use.
// @module: nodenext
// @filename: lib/index.d.ts
declare const configs: { a: 1 };
export { configs };
// @filename: m.d.cts
declare const x: typeof import("./lib/index.js");
export = x;
// @filename: p.d.ts
declare const y: typeof import("./lib/index.js");
export = y;
// @filename: index.ts
import * as M from "./m.cjs";
import P = require("./p.js");
const r: number = M.configs;
const s: number = P.configs;
const t: 2 = P.configs.a;
