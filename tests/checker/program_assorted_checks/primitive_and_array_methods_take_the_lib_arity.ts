// The built-in member models take the lib's parameter lists: a fixed list of
// optional parameters is no rest parameter, so an extra argument is TS2554.
declare const s: string;
declare const n: number;
declare const b: bigint;
declare const xs: number[];
s.charAt(1, 2);
s.padStart(1, "a", "b");
s.split(",", 1, 2);
n.toString(10, 2);
n.toFixed(1, 2);
n.toLocaleString("en", {}, 1);
b.toString(10, 2);
xs.join(",", 1);
xs.slice(1, 2, 3);
xs.sort(undefined, 1);
xs.toLocaleString("en");
const ok: string = s.charAt(0) + s.padEnd(2) + n.toFixed() + xs.join() + xs.slice(1).length;
