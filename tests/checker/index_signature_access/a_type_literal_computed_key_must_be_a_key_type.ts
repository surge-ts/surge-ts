// @strict: true
// A type literal's computed member name is checked as an expression, as an
// interface's is: an enum object, an object or a boolean is no key type
// (TS2464 at the bracketed name).
export namespace Foo {
  export enum Enum {
    A = "a",
    B = "b",
  }
}
declare const obj: { a: 1 };
declare const flag: boolean;
export type T1 = { x?: { [Foo.Enum]: 0 } };
export interface I1 { [Foo.Enum]: 0 }
type T2 = { [obj]: 0 };
type T3 = { [flag](): void };
let v: { [Foo.Enum]: 0 };
