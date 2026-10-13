// @declaration: true
// @module: commonjs
// @target: es2015
// @filename: bug.ts
export const SYMBOL = Symbol()

export interface Interface {
  readonly [SYMBOL]: string;
}

export function createInstance(): Interface {
  return {
    [SYMBOL]: ''
  }
}
// @filename: type.ts
namespace Foo {
  export const sym = Symbol();
}
export type Type = { x?: { [Foo.sym]: 0 } };
// @filename: index.ts
import { createInstance } from './bug'
import { type Type } from "./type";

export const spread = {
  ...createInstance(),
}
export const foo = { ...({} as Type) };
const local = { ...createInstance() };
export const plain = createInstance();
