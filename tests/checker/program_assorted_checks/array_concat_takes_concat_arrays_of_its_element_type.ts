// `concat(...items: ConcatArray<T>[])` and `concat(...items: (T |
// ConcatArray<T>)[])`: an item of another element type matches neither.
declare const numbers: number[];
declare const strings: string[];
declare const readonlyNumbers: readonly number[];
declare const pair: [number, number];
numbers.concat(numbers, readonlyNumbers, pair, 1);
numbers.concat(strings);
numbers.concat("a");
numbers.concat([Symbol()]);
const joined: number[] = numbers.concat([1], 2);
