// @strict: true
// @module: esnext
// @moduleResolution: bundler
// @filename: component.d.html.ts
declare const html: string;
export default html;
// @filename: main.ts
// `./component.html` finds `component.d.html.ts`, an extension
// `GetResolutionDiagnostic` refuses without `allowArbitraryExtensions`: the
// import is TS6263 and resolves to nothing.
import html from "./component.html";
export const n: number = html;
