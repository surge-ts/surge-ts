// @module: commonjs
// @esModuleInterop: true
// @filename: a.ts
export = {}
// @filename: c.ts
export const c = 1;
// @filename: b.ts
export * as ns from './a';
export * as ok from './c';
export * from './a';
