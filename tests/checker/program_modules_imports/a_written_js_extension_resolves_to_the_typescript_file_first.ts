// @filename: b.ts
export const fromTs = 1;
// @filename: b.js
export const fromJs = 1;
// @filename: main.ts
import { fromTs, fromJs } from "./b.js";
