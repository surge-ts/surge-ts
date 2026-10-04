// @isolatedModules: true
// @module: esnext
// @filename: type.ts
export type T = number;
export const V = 1;
// @filename: plain.ts
import { T } from "./type";
export default T;
// @filename: value.ts
import { V } from "./type";
export default V;
