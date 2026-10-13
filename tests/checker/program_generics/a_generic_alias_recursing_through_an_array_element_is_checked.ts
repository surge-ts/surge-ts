// tsc defers a type reference in an array element, a tuple element, or a type
// argument of an interface, so a generic alias may recur there.
type RecArray<T> = Array<T | RecArray<T>>;
type NestedList<T> = T | NestedList<T>[];
type Pair<T> = [T, Pair<T>?];

const a: RecArray<string> = [1, ["a"]];
const b: RecArray<string> = [["a", 2]];
const c: NestedList<string> = [1];
const d: Pair<string> = ["a", [2]];
declare const n: number;
const e: string | RecArray<string> = n;

declare function flat<T>(a: RecArray<T>): T[];
flat([1, ["a"]]);
const ok: RecArray<number> = [1, [2, [3]]];
