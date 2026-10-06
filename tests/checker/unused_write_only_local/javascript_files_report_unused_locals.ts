// @allowJs: true
// @checkJs: true
// @noUnusedLocals: true
// @noUnusedParameters: true
// @filename: types.d.ts
export interface Foo { a: number; }
// @filename: esm.js
/** @import { Foo } from './types' */
class Used {}
class Unused {}

function f1() { return undefined; }

/** @param {Used} u @returns {Foo} */
export function f2(u, extra) { return /** @type {Foo} */ (u); }

/** @type {typeof Helper} */
export const h = null;
function Helper() {}
// @filename: script.js
var globalish = 1;
function inner() {
    const local = 1;
}
