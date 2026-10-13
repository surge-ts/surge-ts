// @filename: foo.d.ts
export var x: number;
export function fn(): void;
export interface Thing { n: number }
export as namespace Foo;
// @filename: a.ts
Foo.fn();
let x: Foo.Thing;
let y: Foo.Other;
let z: number = x.n;
