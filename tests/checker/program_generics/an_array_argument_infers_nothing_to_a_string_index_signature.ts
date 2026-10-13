// An array has no string index and no inferable one, so it gives a string
// index signature's type parameter no candidate: that parameter is `unknown`,
// and an `unknown` the declaration writes is a type like any other.
interface Dictionary<T> { [key: string]: T; }
declare const numbers: number[];
declare function values<T>(dictionary: Dictionary<T>): T[];
values(numbers);
declare function keys<T>(dictionary: { [key: string]: T }): string[];
keys(numbers);
declare function record(dictionary: { [key: string]: unknown }): void;
record(numbers);

declare function all<T>(list: T[], f: (x: T) => string): boolean;
declare function all<T>(list: Dictionary<T>, f: (x: T) => string): boolean;
all(numbers, (x: number) => 1);

declare const o: { b: 1 };
declare function field<T>(x: { a: T }): void;
field(o);
declare function first<T>(list: ArrayLike<T>): T;
const f: number = first(numbers);
