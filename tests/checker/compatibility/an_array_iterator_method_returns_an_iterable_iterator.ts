// An array's `values()`, `keys()` and `entries()` return the lib's
// `ArrayIterator<T>`, which inherits `[Symbol.iterator]` from `IteratorObject`.
declare const list: Array<number>;
declare const numbers: number[];
declare const pair: [number, string];
const iterable: Iterable<number> = list.values();
const entries: IterableIterator<[number, number]> = numbers.entries();
const keys: Iterable<number> = pair.keys();
function drain<T>(iterator: IterableIterator<T>): T[] {
    return [...iterator];
}
const drained: number[] = drain(numbers.values());
const wrong: Iterable<string> = numbers.values();
export { iterable, entries, keys, drained, wrong };
