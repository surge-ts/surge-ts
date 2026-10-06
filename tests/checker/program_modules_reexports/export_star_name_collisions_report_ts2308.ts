// @filename: a.ts
export class A {}
export class B {}
export type T = number;
// @filename: b.ts
export class B {}
export class C {}
export const T = 1;
// @filename: shared.ts
export const S = 1;
// @filename: d.ts
export { S } from "./shared";
// @filename: e.ts
export { S } from "./shared";
// @filename: c.ts
export type * from "./a";
export * from "./b";
// @filename: f.ts
export * from "./d";
export * from "./e";
// @filename: g.ts
export * from "./a";
export * from "./b";
export const B = 2;
export const T = 3;
