// @noImplicitAny: true
type Call = (x: number) => number;
interface Shape { size: number }
const a: Call<string> = x => x;
const b: Shape<number> = { size: "big" };
const c: Call = y => y;
