// @module: esnext
// @moduleResolution: bundler
// @strict: true
// @filename: node_modules/pkg/package.json
{ "name": "pkg", "version": "1.0.0", "types": "types/index.d.ts" }
// @filename: node_modules/pkg/types/index.d.ts
export interface Root { root: string }
// @filename: node_modules/pkg/types/other.d.ts
export interface Other { other: string }
// @filename: src/imported.ts
// The augmentation's name resolves, but only an import adds the file it
// names to the program: the first is TS2664, the second merges.
import type { Root } from "pkg/types/index";
declare module "pkg/types/index" {
  interface Root { extra: number }
}
export const r: Root = { root: "", extra: 1 };
// @filename: src/augmented.ts
export {};
declare module "pkg/types/other" {
  interface Other { extra: number }
}
