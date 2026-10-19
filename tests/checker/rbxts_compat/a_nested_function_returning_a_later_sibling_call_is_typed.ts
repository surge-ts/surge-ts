// A function declared in a body whose return calls a sibling declared after
// it returns what that sibling does: both are hoisted, and the sibling's
// return is known when the first one's is read.
function outer() {
	function read() {
		return value();
	}
	async function value() {
		return 42;
	}
	const r: boolean = read();
}
function chain() {
	function first() {
		return second();
	}
	function second() {
		return third();
	}
	function third() {
		return "s";
	}
	const f: boolean = first();
}
function cycle() {
	function a(n: number): number {
		return n > 0 ? b(n - 1) : 0;
	}
	function b(n: number) {
		return a(n);
	}
	const c: boolean = b(1);
}
