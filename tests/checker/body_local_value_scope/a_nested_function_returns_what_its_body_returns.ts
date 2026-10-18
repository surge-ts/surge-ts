// A function declared in a function body, written without a return type,
// returns what its body returns, as a top-level one does: its calls are typed,
// reading what is in scope where it is declared (the enclosing parameters,
// the module, functions declared before it).
export {};
declare function wrap<T>(value: T): { got: T };
function outer(prefix: string) {
	function plain() {
		return 1;
	}
	function generic() {
		return wrap(1);
	}
	function closing() {
		return prefix.length;
	}
	function earlier() {
		return "text";
	}
	function calling() {
		return earlier();
	}
	const a: string = plain();
	const b: string = generic();
	const c: string = closing();
	const d: number = calling();
}
