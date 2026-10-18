// @noLib: true
// Under `noLib` the program's own global `Array` is the array type, as
// roblox-ts's `@rbxts/compiler-types` declares it: `Array<T>` is `T[]`, its
// members are the ones it declares or inherits (`size()` from `ArrayLike`),
// and its constructor's generic overloads construct what they declare.
// @filename: lib.d.ts
interface ArrayLike<T> {
	size(): number;
	readonly [index: number]: T;
}
interface ReadonlyArray<T> extends ArrayLike<T> {
	join(separator?: string): string;
}
interface Array<T> extends ReadonlyArray<T> {
	length: number;
	push(...items: T[]): number;
}
interface ArrayConstructor {
	new <T>(): Array<T>;
	new <T>(length: number, value?: T): Array<T>;
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
const literal = [1];
const inherited: string = literal.size();
const joined: number = literal.join();
declare const frozen: ReadonlyArray<number>;
const frozenSize: string = frozen.size();
declare const written: Array<number>;
const sameArray: number[] = written;
const wrongElement: string[] = written;
const constructed = new Array<string>();
const notNumber: number = constructed;
const filled = new Array<number>(3, 0);
const notString: string = filled;

interface Box<T> {
	value: T;
}
interface BoxConstructor {
	new <T>(): Box<T>;
	new <T>(value: T): Box<T>;
}
declare const Box: BoxConstructor;
const box: number = new Box<string>();
