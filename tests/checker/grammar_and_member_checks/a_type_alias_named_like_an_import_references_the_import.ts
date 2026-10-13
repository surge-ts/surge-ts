// @filename: f1.ts
export enum E {X}
export interface I { a: number }
// @filename: f2.ts
import {E, I} from "./f1";
export type E = E;
export type I = I;
type Own = Own;
