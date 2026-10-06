enum Two { X, Y, Z }
type Narrow = { k: Two.X, v: string } | { k: Two.Y, v: string };
declare const t: Two;
const n: Narrow = { k: t, v: "" };
type U = { a: string, b: number } | { a: number, b: number };
const u: U = { a: true, b: 1 };
const w: U = { a: true, b: "x" };
