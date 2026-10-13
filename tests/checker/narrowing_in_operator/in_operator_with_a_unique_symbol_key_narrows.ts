const sym = Symbol();
const other = Symbol();
const k = "a";
function f(x: object) {
    if ("a" in x && 1 in x && sym in x) {
        x.a;
        x[1];
        x[sym];
        x[other];
    }
}
function g(x: { b: number } | { a: string }) {
    if (k in x) {
        const s: string = x.a;
    }
}
