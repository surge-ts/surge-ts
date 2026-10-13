// @allowJs: true
// @checkJs: true
// @module: commonjs
// @filename: x.js
export const v = 1;
// @filename: a.js
export const used = 1;
/** @import * as f from "./x.js" with { type: "json" } */
/** @import * as g from "./x.js" with */
/** @import * as h from "./x.js" with { "resolution-mode": "import" } */
/** @import * as i from "./x.js" with {} */
