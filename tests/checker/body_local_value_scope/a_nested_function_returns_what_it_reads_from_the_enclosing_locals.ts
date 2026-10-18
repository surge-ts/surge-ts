// A nested function's return is read from its body with the enclosing body's
// locals in scope, as tsc's closure sees them: a function hoisted before
// `const values = [...]` runs, or one that calls a sibling declared after it,
// still returns what reading them gives.
declare function pair<T extends Array<unknown>>(...values: T): T;
export function outer() {
	const values = [1, 2, 3, 4];
	function read(enabled: boolean) {
		if (enabled) {
			return pair(values.pop(), values.pop());
		}
		return pair(0, values.pop());
	}
	const first: string = read(true);
	function lengths() {
		return names.map((name) => name.length);
	}
	const names = ["a", "bb"];
	const counted: string = lengths();
	function forward() {
		return later();
	}
	function later() {
		return 1;
	}
	const viaLater: string = forward();
}
