// A declaration annotated with a union, or with a union enum, reads as what its
// initializer narrowed that type to, at module scope as in a block.
enum Kind { A, B }
declare enum Mixed { ONE, TWO, THREE = "x" }

const kind: Kind = Kind.A;
const onlyA: Kind.A = kind;
const mixed: Mixed = Mixed.ONE;
const one: Mixed.ONE = mixed;
let literal: 1 | 2 = 1;
const narrowedLiteral: 1 = literal;
let text: string | number = "a";
const narrowedText: string = text;
const wrong: Kind.B = kind;

function inBlock() {
    const k: Kind = Kind.A;
    const a: Kind.A = k;
    let l: Kind = Kind.B;
    const b: Kind.B = l;
    const c: Kind.A = l;
}

function readsDeclared() {
    const fromFunction: 1 = literal;
}
