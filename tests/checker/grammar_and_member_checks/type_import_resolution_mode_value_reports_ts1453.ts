// @module: node16
// @target: es2022
// @filename: a.ts
export interface A { x: number }
// @filename: b.ts
import type { A } from "./a" with { "resolution-mode": "foobar" };
export type B = A;
