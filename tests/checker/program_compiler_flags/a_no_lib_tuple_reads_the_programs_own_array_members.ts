// @noLib: true
// Under `noLib` a tuple is an array of its elements' union, so it has the
// members the program's own `Array` declares or inherits (`size()` from
// `ArrayLike`, as roblox-ts's `@rbxts/compiler-types` declares it), and so
// does an intersection with a tuple (`LuaTuple<T> = T & { … }`).
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
type LuaTuple<T extends Array<any>> = T & { readonly LUA_TUPLE: void };
declare const pair: [number, string];
const pairSize: string = pair.size();
const pairJoined: number = pair.join();
declare const rest: [number, ...string[]];
const restSize: string = rest.size();
declare const tuple: LuaTuple<[number, number, number]>;
const tupleSize: string = tuple.size();
const tupleJoined: number = tuple.join(",");
