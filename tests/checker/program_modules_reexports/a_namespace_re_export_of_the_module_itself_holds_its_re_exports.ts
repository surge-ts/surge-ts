// @target: es2022
// @module: esnext
// @filename: self.ts
const someValue = "someValue";
export { someValue as x };
export { x as y } from "./self";
export * as z from "./self";
import { z as valueZ } from "./self";
if (valueZ["x"] !== "someValue") throw "";
if (valueZ["y"] !== "someValue") throw "";
valueZ.y;
valueZ.z;
valueZ.w;
