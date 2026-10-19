// @noLib: true
// @surge-compare: messages
// `LuaTuple<[string, number]>` and `LuaTuple<Array<string | number>>` merge to
// the same members (`string | number` by number), but are different
// intersections: an instantiation over one, read after one over the other, is
// still its own tuple.
// @filename: lib.d.ts
interface ArrayLike<T> {
	readonly [index: number]: T;
}
interface ReadonlyArray<T> extends ArrayLike<T>, Iterable<T> {
	size(): number;
}
interface Array<T> extends ReadonlyArray<T> {
	[index: number]: T;
}
interface IteratorResult<T> {
	done: boolean;
	value: T;
}
interface Iterator<T> {
	next(): IteratorResult<T>;
}
interface Iterable<T> {
	[Symbol.iterator](): Iterator<T>;
}
interface IterableFunction<T> extends Iterable<T> {
	(): T;
}
interface SymbolConstructor {
	readonly iterator: unique symbol;
}
declare const Symbol: SymbolConstructor;
interface Boolean {}
interface Function {}
interface IArguments {}
interface Number {}
interface Object {}
interface RegExp {}
interface String {}
interface CallableFunction {}
interface NewableFunction {}
type LuaTuple<T extends Array<any>> = T & {
	readonly _nominal_LuaTuple: unique symbol;
};
declare namespace string {
	function gmatch(str: string, pattern: string): IterableFunction<LuaTuple<Array<string | number>>>;
	function gsub(str: string, pattern: string, repl: string): LuaTuple<[string, number]>;
}

// @filename: index.ts
export {};
declare const captures: Array<LuaTuple<Array<string | number>>>;
declare const replaced: Array<LuaTuple<[string, number]>>;
const [text, count] = replaced[0];
const first: string = text;
const second: number = count;
const read: string = replaced[0][0];
const wrong: boolean = replaced[0][1];

declare const pairs: IterableFunction<LuaTuple<[number, number]>>;
declare const numbers: IterableFunction<LuaTuple<Array<number>>>;
const [low, high] = pairs();
const sum: number = low + high;
const each: string = numbers()[2];
