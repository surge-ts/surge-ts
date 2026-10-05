// @allowJs: true
// @checkJs: true
// @module: esnext
// @filename: 0.d.ts
export interface I {}
// @filename: 1.js
/** @import { I } from "./0" with { type: "json" } */
/** @import * as foo from "./0" with { type: "json" } */

/** @param {I} a */
function f(a) {}
