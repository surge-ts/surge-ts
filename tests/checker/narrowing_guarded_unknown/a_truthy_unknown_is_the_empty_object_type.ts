// @strict: true
declare function obj(x: object): void;
function f1(x: unknown) {
    if (!x) { return; }
    const a: 7 = x;
    if (typeof x === 'object') { obj(x); }
}
function f2(x: unknown) {
    if (x) { x.foo; }
}
