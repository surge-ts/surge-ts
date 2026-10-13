// @module: commonjs
// @filename: node_modules/foo/index.d.ts
export = foo;
declare namespace foo {
    export type T = number;
}
// @filename: node_modules/bar/index.d.ts
export type U = string;
// @filename: a.ts
import * as foo from "foo";
declare module "foo" {
    export function f(): T;
}
// @filename: b.ts
import * as bar from "bar";
declare module "bar" {
    export function g(): U;
    export function h(): Missing;
}
