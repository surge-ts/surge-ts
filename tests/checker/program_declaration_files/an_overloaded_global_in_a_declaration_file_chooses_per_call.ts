// An overloaded function a declaration file declares globally is its group,
// each member as written: a call in another file picks the member its
// arguments fit, a generic one instantiated, as `chooseOverload` does.
// @filename: globals.d.ts
type LuaTuple<T extends Array<any>> = T & { readonly LUA_TUPLE: never };
declare function describe(value: string): { text: true };
declare function describe(value: number): { count: true };
declare function wrap<T>(value: T[]): { many: T };
declare function wrap<T>(value: T): { one: T };
declare function pcall<T extends Array<any>, U extends Array<any>>(
	func: (...args: T) => LuaTuple<U>,
	...args: T
): LuaTuple<[true, ...U] | [false, ...{ [K in keyof U]: K extends "0" ? unknown : undefined }]>;
declare function pcall<T extends Array<any>, U>(func: (...args: T) => U, ...args: T): LuaTuple<[true, U] | [false, unknown]>;

// @filename: index.ts
const counted: string = describe(1);
const texted: number = describe("a");
const one: number = wrap(1);
const many: number = wrap([1]);
const result: string = pcall(() => 1);
export function f() {
	const [ok, value] = pcall(() => true);
	const flag: string = ok;
	const read: string = value;
	const [success] = pcall((x: number) => x > 0, 1);
	const done: string = success;
}
