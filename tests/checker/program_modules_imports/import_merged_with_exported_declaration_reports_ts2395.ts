// @module: commonjs
// @filename: assignment.ts
export default {
    foo: 12
};
// @filename: types.ts
export interface Shape { a: number }
export const value = 1;
// @filename: user.ts
import Obj from "./assignment";
export const Obj = void Obj;
// @filename: user2.ts
import { Shape } from "./types";
export const Shape = 1;
// @filename: user3.ts
import { value } from "./types";
const value = 2;
export {};
// @filename: f1.ts
export function f() {}
export namespace N { export var x = 1; }
// @filename: user4.ts
import { f, N } from './f1';
export function f() {}
export namespace N {
    export interface I { x: any }
}
// @filename: user5.ts
import * as ns from './f1';
export class ns {}
