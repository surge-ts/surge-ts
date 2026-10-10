// @strict: true
// A rest parameter typed by a fixed tuple stands for one parameter per
// element, its optional elements trailing; one written as an array pattern
// is typed by the tuple the pattern implies.
function d2(...args: [number, string]) { }
d2(1);
d2(1, "", 3);
function d3(...args: [number, string?]) { }
d3();
d3(1);
d3(1, "", 3);
function d4(...[a, b]: [number, string]) { }
d4(1);
declare function g4(...args: readonly [number, string?]): void;
g4();
type P1 = Parameters<(a?: number, b?: string) => void>;
declare function g1(...args: P1): void;
g1();
declare function h<F extends (...args: any[]) => any>(f: F, ...args: Parameters<F>): void;
h((a?: number) => {});
function r1(...[[k1, v1], [k2, v2]]: any[]) { }
function r2(...[[k1, v1], [k2, v2]]) { }
r2([1, 2]);
r2([1, 2], [3, 4], [5, 6]);
const f = (...[x, y]) => x;
f(1);
