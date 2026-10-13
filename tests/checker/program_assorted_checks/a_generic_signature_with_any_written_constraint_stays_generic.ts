// A signature's type parameters are rendered into its head, which is what
// marks it generic: a union, intersection, array, `keyof` or object
// constraint used to drop the head and leave a plain signature over erased
// parameters that accepted anything.
declare const narrow: <T extends "a">(x: T) => T;
const wider: <T extends "a" | "b">(x: T) => T = narrow;
declare const literal: (x: "a") => void;
const unionTarget: <T extends "a" | "b">(x: T) => void = literal;
const numberTarget: <T extends string | number>(x: T) => void = 1;
const intersectionTarget: <T extends string & {}>(x: T) => void = "s";
function declared<T extends string | number>(x: T): void {}
const fromDeclaration: typeof declared = 1;
const objectConstraint: <T extends { a: 1 | 2 }>(x: T) => void = narrow;
function take(f: <T extends string>(x: T) => void) {}
take(literal);
