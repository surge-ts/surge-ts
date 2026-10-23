// @noLib: true
// @typeRoots: node_modules/@rbxts
// @types: compiler-types
// roblox-ts's `Signal<T>` keeps its listeners in the `Set` its compiler types
// declare: a member constructing it over the class's type variable reads the
// dependency's members, so the disconnect callback returns `boolean`.
// @filename: node_modules/@rbxts/compiler-types/package.json
{ "name": "@rbxts/compiler-types", "version": "1.0.0", "types": "core.d.ts" }
// @filename: node_modules/@rbxts/compiler-types/core.d.ts
interface Array<T> {
	readonly [index: number]: T;
}
interface ReadonlyArray<T> {
	readonly [index: number]: T;
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
interface ReadonlySet<T> {
	has(value: T): boolean;
}
interface Set<T> extends ReadonlySet<T> {
	add(this: Set<T>, value: T): this;
	delete(this: Set<T>, value: T): boolean;
}
interface SetConstructor {
	new <T>(): Set<T>;
	new <T>(values: ReadonlyArray<T>): Set<T>;
}
declare const Set: SetConstructor;
// @filename: index.ts
export class Signal<T> {
	private listeners = new Set<(value: T) => void>();
	connect(listener: (value: T) => void) {
		this.listeners.add(listener);
		return () => this.listeners.delete(listener);
	}
	read() {
		const all: number = this.listeners;
	}
}
declare const signal: Signal<string>;
const disconnect: number = signal.connect(() => {});
const removed: number = signal.connect(() => {})();
