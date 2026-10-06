// @allowJs: true
// @checkJs: true
// @filename: types.ts
export interface Foo { a: number; }
export interface Bar { b: string; }
// @filename: a.js
/** @import { Foo } from "./types" */
/** @import { Foo, Bar } from "./types" */
/** @import * as T from "./types" */
/** @import { Bar as T } from "./types" */

/** @typedef {number} Klass */
class Klass { }

/** @typedef {string} Twice */
/** @typedef {number} Twice */

/** @typedef {number} Fine */
function Fine() {}

/** @param {Foo} foo */
function f(foo) {}
export { f, Klass, Fine };
