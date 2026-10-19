// @noLib: true
// Under `noLib` the program's own `String` is a string's apparent type: a
// member it declares (roblox-ts's Luau `sub(i, j)`) stands over the built-in
// one of the same name (`String.prototype.sub()`).
// @filename: lib.d.ts
interface String {
	sub(this: string, i: number, j?: number): string;
	size(this: string): number;
	readonly [n: number]: string;
}
interface Array<T> {
	length: number;
}
interface Boolean {}
interface Function {}
interface IArguments {}
interface Number {}
interface Object {}
interface RegExp {}
interface CallableFunction {}
interface NewableFunction {}

// @filename: index.ts
export {};
const first: string = "Hello".sub(1, 1);
const text = "Hello";
const head: number = text.sub(1);
const size: string = text.size();
function getText() {
	return "hello";
}
const middle: string = getText().sub(2, 3);
