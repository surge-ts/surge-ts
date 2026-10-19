// A generic function declared in a body returns, per call, what its own body
// returns, read where it is declared: a type that body declares beside it
// (roblox-ts's `make<defined>().callback` over a local `Callable<T>`).
export function outer() {
	interface Box {
		value: number;
	}
	type Callable<T> = (this: T, value: number) => number;
	function boxed<T>() {
		const box: Box = { value: 1 };
		return box;
	}
	const explicit: string = boxed<number>();
	function inferred<T>(value: T) {
		const box: Box = { value: 1 };
		return box;
	}
	const fromArgument: string = inferred(1);
	function make<T extends {}>() {
		const callback: Callable<T> = (value) => value + 1;
		return { callback };
	}
	const member: string = make<{}>().callback;
}
