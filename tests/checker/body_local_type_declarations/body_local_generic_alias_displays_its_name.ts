// A body-local generic alias is displayed by its name and arguments
// (`Callable<void>`), as a top-level one is, not by the position surge keys
// it with.
export function f() {
	type Callable<T> = (this: T, value: number) => number;
	type Pair<T> = { first: T; second: T };
	const callback: Callable<void> = (value) => value + 1;
	const pair: Pair<number> = { first: 1, second: 2 };
	const fromCallback: string = callback;
	const fromPair: string = pair;
}
