// @declaration: true
// @strict: true
// @filename: type.ts
export type Nested = { x?: { [Missing.A]: 0 } };
export type Flat = { [Missing.A]: 0 };
export interface Iface { [Absent]: 0 }
const local: Iface = {};
// @filename: index.ts
import { type Nested, type Flat } from "./type";
export const spread = { ...({} as Nested) };
export const flat = {} as Flat;
const empty: Flat = {};
