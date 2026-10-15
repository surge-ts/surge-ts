// @strict: true
interface Dictionary<T> { [x: string]: T; }
declare function all<T>(list: Dictionary<T>, f?: (v: T) => boolean): T;
declare const arr: (string | number)[];
const result = all(arr, (v) => true);
const typed: [] = all({ a: 1 }, (v) => { const n: number = v; return true; });
declare function identity<T>(value: T): T;
interface Iterator_<T, U> { (value: T, index: number): U; }
declare function every<T>(list: T[], iterator?: Iterator_<T, boolean>): boolean;
every([true, 1, 'yes'], identity);
declare function map<T, U>(f: (x: T) => U, xs: T[]): U;
const mapped: string = map(identity, [1]);
