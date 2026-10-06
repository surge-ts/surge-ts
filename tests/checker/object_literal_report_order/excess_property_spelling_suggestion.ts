// An excess property written as an identifier that misspells a member of the
// target is TS2561 naming that member; a string-literal key, or a name with no
// close member, stays TS2353.
let b: { __foo: number };
b = { ___foo: 100 };
b = { "___foo": 100 };
b = { unrelated: 100 };
const c: { color: string; width: number } = { colour: "red", width: 1 };
type Shape = { kind: "circle"; radius: number } | { kind: "square"; size: number };
const s: Shape = { kind: "circle", radios: 1 };
