// `checkModuleDeclaration` checks a block's declarations as a file's,
// including a variable no export publishes.
// @module: commonjs
// @filename: a.ts
declare namespace Outer { const v: number; }
declare module "m" {
    import Alias = Outer;
    var w: Alias;
    export var x: Alias;
    interface I { a: Outer }
    type T = Outer;
}
