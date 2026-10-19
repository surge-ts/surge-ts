// Calling what a call returns, when that is an interface with a call
// signature (`"Hello".gmatch(".")()` over roblox-ts's
// `IterableFunction<T> { (): T }`), returns the signature's type.
interface Callable {
	(): number;
}
declare function make(): Callable;
const fromFunction: boolean = make();
const called: boolean = make()();
declare const holder: { method(): Callable; property: () => Callable };
const fromMethod: boolean = holder.method()();
const fromProperty: boolean = holder.property()();
export {};
