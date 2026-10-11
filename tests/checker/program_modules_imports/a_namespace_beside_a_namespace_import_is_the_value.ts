// @module: commonjs
// A namespace import has value meaning whatever its module exports, so a
// local namespace of the same name conflicts with it (TS2440) and is what the
// name reads; its members, not the module's, are reached.
// @filename: file1.ts
export namespace Library {
    export type Bar = { a: number };
}
// @filename: file2.ts
import * as Lib from './file1';
namespace Lib {
    export const foo: string = "";
}
Lib.foo;
var x: Lib.Bar;
export { Lib }
