// @allowJs: true
// @checkJs: true
// @filename: self.js
/** @import * as self from "./self" */
// @filename: importer.js
/** @import * as other from "./self" */
export {};
// @filename: typed.js
/** @import { Y } from "./types" */
/** @type {Y} */
var value = 1;
// @filename: types.ts
export type Y = string;
