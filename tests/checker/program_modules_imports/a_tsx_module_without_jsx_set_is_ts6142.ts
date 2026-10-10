// @module: commonjs
// A specifier that resolves to a `.tsx` file while `jsx` is unset is TS6142,
// beside TS2306 when the file is not a module.
// @filename: comp.tsx
export const comp = 1;
// @filename: scr.tsx
const script = 1;
// @filename: a.ts
import { comp } from "./comp";
import scr from "./scr";
import * as ns from "./comp";
import eq = require("./scr");
console.log(comp, scr, ns, eq);
