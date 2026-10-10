// @target: es2015
// @module: none
// tsc rejects `module: none` as it reads the config (TS6046) and leaves the
// option unset, so the module kind follows the target: ES2015, which has no
// dynamic import (TS1323). The rejection stops nothing else being checked.
// @filename: a.ts
const foo = import("./b");
const n: number = "s";
export {};
// @filename: b.ts
export default 1;
