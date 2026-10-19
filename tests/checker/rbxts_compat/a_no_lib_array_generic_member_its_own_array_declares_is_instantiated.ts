// @noLib: true
// @surge-compare: messages
// A generic member only the program's own `Array` declares (roblox-ts's
// `mapFiltered<U>`) is instantiated for the call, as a declared member's is.
// @filename: lib.d.ts
type NonNullable<T> = T & {};
interface ReadonlyArray<T> {
	readonly [index: number]: T;
	mapFiltered<U>(this: ReadonlyArray<{}>, callback: (value: T, index: number) => U): Array<NonNullable<U>>;
	firstOf<U>(callback: (value: T) => U): U;
}
interface Array<T> extends ReadonlyArray<T> {
	length: number;
}
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
declare const values: Array<number>;
const halves = values.mapFiltered((value) => (value > 0 ? value / 2 : undefined));
const fromHalves: string = halves;
const first: boolean = values.firstOf((value) => `${value}`);
