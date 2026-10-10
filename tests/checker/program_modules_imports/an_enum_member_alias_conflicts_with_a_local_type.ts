// @strict: true
// `import x = E.M` naming an enum member brings a value and a type: a local
// type of the same name conflicts with it (TS2440), and the exported type and
// the local import must agree on being exported (TS2395 on both).
// @filename: enum.ts
export enum Enum {
    A,
    B
}
// @filename: alias.ts
import { Enum } from "./enum";

import EnumA = Enum.A;

export type EnumA = [string] | [string, number];
