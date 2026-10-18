// A nested function's written `this` parameter is part of its signature, as a
// top-level declaration's is: a call with no receiver is checked against it.
export function outer() {
	function needsText(this: string, value: number) {
		return value;
	}
	function allowsVoid(this: string | void, value: number) {
		return value;
	}
	needsText(1);
	allowsVoid(2);
	const viaTypeof: typeof needsText = needsText;
	viaTypeof(3);
}
