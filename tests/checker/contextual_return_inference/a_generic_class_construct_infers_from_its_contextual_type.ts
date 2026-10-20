// @strict: true
// `new C(…)` infers the class's type parameters from its contextual type too,
// as the signature's return `C<T>`, so a callback argument is typed by it.
declare class Box<T> {
    constructor(value?: (x: T) => void);
    value: T;
}
export const typed: Box<string> = new Box((x) => {
    const n: number = x;
});
export function returned(): Box<boolean> {
    return new Box((x) => {
        const s: string = x;
    });
}
class Pair<K, V> {
    constructor(public f: (k: K) => V) {}
}
export const pair: Pair<string, number> = new Pair((k) => k.length);
