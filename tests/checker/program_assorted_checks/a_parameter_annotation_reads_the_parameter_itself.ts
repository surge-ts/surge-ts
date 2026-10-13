// A type literal member of a parameter's annotation may query the parameter
// itself (`b: { y: typeof b }`), which shadows a module value of that name.
export function b() {}
function g(b: { y: typeof b }) {
    return b.y();
}
function h(x: number, a: { y: typeof a }) {
    const n: number = a.y;
}
function k(a: string, c: typeof a) {
    const z: number = c;
}
