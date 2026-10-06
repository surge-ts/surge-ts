// @filename: a.ts
export type { A } from './b';
// @filename: b.ts
export type { A } from './a';
