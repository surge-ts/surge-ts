// @allowJs: true
// @filename: dir.js
export const fromFile = 1;
// @filename: dir/index.ts
export const fromIndex = 1;
// @filename: main.ts
import { fromFile, fromIndex } from "./dir";
