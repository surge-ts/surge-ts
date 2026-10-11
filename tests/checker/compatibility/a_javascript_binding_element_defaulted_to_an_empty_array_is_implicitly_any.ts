// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: a.js
export function nested({ a: { json = [] } = {} } = {}) { json; }
export function flat({ json = [] } = {}) { json; }
export function bare({ json = [] }) { json; }
export function element([json = []] = []) { json; }
export function nullish({ n = null } = {}) { n; }
export function provided({ json = [] } = { json: ["a"] }) { json; }
/** @param {{ json?: string[] }} [config] */
export function annotated({ json = [] } = {}) { json; }
