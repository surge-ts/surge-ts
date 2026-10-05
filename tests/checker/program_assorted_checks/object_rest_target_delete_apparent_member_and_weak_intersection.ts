let o = { a: 1, b: "" };
let a: number;
let rest: { b: string } = { b: "" };
({a, ...rest.b + rest.b} = o);
delete Array.toString;
interface X { x: string }
interface Y { y: number }
interface Z { z?: boolean }
type XY = X & Y;
const xy: XY = {x: 'x', y: 10};
const z1: Z = xy;
