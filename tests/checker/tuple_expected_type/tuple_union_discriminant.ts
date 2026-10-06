// A tuple relates to a union of tuples through its discriminating elements,
// and an array literal is typed as a tuple against such a union even when no
// single member takes it.
type A = ["a", number] | ["b", number] | ["c", string];
declare const b: "a" | "b";
declare const t: ["a" | "b", number];
declare const cond: boolean;

const a1: A = t;
const a2: A = [b, 1];
const a3: A = b === "a" || b === "b" ? [b, 1] : ["c", ""];
const a4: A = cond ? [b, 1] : ["c", ""];

const bad1: A = [b, "x"];
declare const u: ["a" | "c", number];
const bad2: A = u;
