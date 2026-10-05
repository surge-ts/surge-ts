function f2(...args: readonly [string, string]) {
    f2('abc', ...args);
}
function f3(...args: [string, string]) {
    f3('abc', ...args);
}
declare const t: [string, string];
declare const rt: readonly [string, string];
function g(a: string, b: string) {}
g('x', ...t);
g('x', ...rt);
