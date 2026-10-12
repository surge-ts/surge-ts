interface CoolArray<E> extends Array<E> { hello: number; }
declare function baz2(...args: CoolArray<number>): void;
declare const ca: CoolArray<number>;
declare const nums: number[];
declare const pair: [number, number];
baz2(...ca);
baz2(...nums);
baz2(...pair);
