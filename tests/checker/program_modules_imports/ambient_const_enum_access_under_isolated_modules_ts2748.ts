// @isolatedModules: true
// @filename: decl.d.ts
declare const enum G { X = 1 }
// @filename: file1.ts
declare const enum E { X = 1 }
export var y = E.X;
export var z = G.X;
export type T = typeof E;
const enum Local { A }
export var w = Local.A;
