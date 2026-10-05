// @verbatimModuleSyntax: true
// @module: commonjs
// @target: esnext
// @moduleResolution: bundler
// @esModuleInterop: true
// @noEmit: true
// @filename: decl.d.ts
declare function esmy(): void;
export default esmy;
export declare function funciton(): void;
// @filename: main.ts
import esmy, { funciton } from "./decl";
import * as esmy2 from "./decl";
import type { funciton as funciton2 } from "./decl";
import { type funciton as funciton3 } from "./decl";
export { funciton as reexported } from "./decl";
export type { funciton as funciton4 } from "./decl";
