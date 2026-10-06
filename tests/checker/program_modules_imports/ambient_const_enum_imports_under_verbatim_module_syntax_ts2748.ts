// @verbatimModuleSyntax: true
// @module: preserve
// @target: esnext
// @filename: node_modules/pkg/index.d.ts
export declare const enum E { A, B, C }
declare global {
    const enum F { A, B, C }
}
// @filename: a.ts
import { E } from "pkg";
import type { E as _E } from "pkg";
console.log(E.A);
F.A;
declare const enum L { X }
L.X;
let v = L;
// @filename: b.ts
export { E } from "pkg";
export type { E as _E } from "pkg";
