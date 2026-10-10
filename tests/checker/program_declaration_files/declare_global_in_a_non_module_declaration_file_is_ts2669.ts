// @filename: g.d.ts
// `declare global` augments nothing in a declaration file that is not a
// module (TS2669); importing that file is TS2306.
declare global { }
// @filename: b.ts
import * as G from "./g";
export {};
