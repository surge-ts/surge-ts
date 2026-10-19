// @noLib: true
// Under `noLib` the program's own `Array` is an array's apparent type: its
// `sort` (a comparator returning `boolean`, as roblox-ts declares it) is what
// a call is checked against.
// @filename: lib.d.ts
interface ArrayLike<T> {
	size(): number;
	readonly [index: number]: T;
}
interface ReadonlyArray<T> extends ArrayLike<T> {
	join(this: ReadonlyArray<{}>, separator?: string): string;
	reduce(
		this: ReadonlyArray<{}>,
		callback: (accumulator: T, currentValue: T, currentIndex: number, array: ReadonlyArray<T>) => T,
	): T;
	reduce<U>(
		this: ReadonlyArray<{}>,
		callback: (accumulator: U, currentValue: T, currentIndex: number, array: ReadonlyArray<T>) => U,
		initialValue: U,
	): U;
}
interface Array<T> extends ReadonlyArray<T> {
	length: number;
	push(...items: T[]): number;
	sort(this: Array<{}>, compareFunction?: (a: T, b: T) => boolean): Array<T>;
}
interface ArrayConstructor {
	new <T>(): Array<T>;
}
declare const Array: ArrayConstructor;
interface Boolean {}
interface Function {}
interface IArguments {}
interface Number {}
interface Object {}
interface RegExp {}
interface String {}
interface CallableFunction {}
interface NewableFunction {}

// @filename: index.ts
export {};
const xs = [3, 1, 2];
const sorted: Array<number> = xs.sort((a, b) => a < b);
const joined: number = xs.join(",");
