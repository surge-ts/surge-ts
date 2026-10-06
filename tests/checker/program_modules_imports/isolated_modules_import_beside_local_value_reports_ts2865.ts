// @module: commonjs
// @isolatedModules: true
// @filename: type.ts
export type T = number;
export interface I {}
export const v = 1;
// @filename: test1.ts
import { T } from "./type";
const T = 0;
export default T;
// @filename: test2.ts
import { T } from "./type";
type T = number;
export default T;
// @filename: test3.ts
import type { T } from "./type";
const T = 0;
export default T;
// @filename: test4.ts
import { type I } from "./type";
const I = 0;
export { I };
// @filename: test5.ts
import { I } from "./type";
function I() {}
I();
// @filename: test6.ts
import { v } from "./type";
type v = string;
export { v };
