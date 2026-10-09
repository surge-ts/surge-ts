// @strict: true
declare function take(a: ArrayLike<bigint>): void;
take([1, 2, 3]);
const strings: ArrayLike<string> = [1, "a"];
interface Indexed { [n: number]: number; x: number }
const missing: Indexed = [1, 2];
const element: Indexed = ["a", 2];
