// A function type written in a body with `this: A`, where `A` is a class the
// body declares and one of A's methods names `A` itself, takes `A` as its
// `this` type: the call without a receiver and the assignment to an
// incompatible `this` are both errors.
export function outer() {
	class A {
		h(this: A) {}
		k(other: A) {}
	}
	const typed: (this: A) => void = () => {};
	typed();
	let withValue!: (this: A, value: number) => void;
	const mismatched: (this: number, value: number) => void = withValue;
	return mismatched;
}
