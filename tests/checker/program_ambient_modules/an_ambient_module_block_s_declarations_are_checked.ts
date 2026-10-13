// @module: commonjs
// @filename: a.ts
declare module "m" {
    export var z: Missing;
    var w: Missing2;
    interface I { a: Missing3 }
    export interface J { a: Missing4 }
    type T = Missing6;
    interface K<P> { p: P; q: J; r: I }
    type U<Q> = Q | T;
}
// @filename: b.ts
export interface Base { }
// @filename: c.ts
export {}
declare module "./b" {
    interface Base { extra: Missing7; ok: Base }
    var hidden: Missing8;
    type Alias = Base;
}
