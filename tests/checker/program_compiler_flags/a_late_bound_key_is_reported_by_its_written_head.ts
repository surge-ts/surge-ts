// @declaration: true
// @module: commonjs
// @target: es2015
// @filename: type.ts
// Emit tracks a late-bound key by the first identifier it is written with
// (`Foo` of `[Foo.sym]`), not by the unique symbol it binds to.
export namespace Foo {
  export const sym = Symbol();
}
export const sym = 1;
export type Type = { x?: { [Foo.sym]: 0 } };
export const literal = { [Foo.sym]: 0 as const };
// @filename: imports_head.ts
import { type Type, Foo } from "./type";
export const named = { ...({} as Type) };
// @filename: imports_other.ts
import { type Type, sym } from "./type";
export const unnamed = { ...({} as Type), sym };
// @filename: local.ts
export const local = (() => {
  const s = Symbol();
  return { [s]: 0 as 0 };
})();
