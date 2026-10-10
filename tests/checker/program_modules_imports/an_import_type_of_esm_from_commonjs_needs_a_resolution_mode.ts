// @module: node18
// @target: es2022
// Under node16/node18 a CommonJS file loads an ES module only through a
// `resolution-mode` attribute: a type-only import without one is TS1541, an
// import type TS1542. `require` mode is accepted, `require(esm)` being
// supported from node18.
// @filename: module.mts
export {};
// @filename: common.cts
import type {} from "./module.mts";
import type {} from "./module.mts" with { "resolution-mode": "import" };
import type {} from "./module.mts" with { "resolution-mode": "require" };
type _1 = typeof import("./module.mts");
type _2 = typeof import("./module.mts", { with: { "resolution-mode": "import" } });
type _3 = typeof import("./module.mts", { with: { "resolution-mode": "require" } });
