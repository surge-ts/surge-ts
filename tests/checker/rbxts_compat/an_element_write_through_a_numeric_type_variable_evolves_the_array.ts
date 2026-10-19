// @noImplicitAny: true
// `isTypeAssignableToKind(…, NumberLike)` reads a type variable through its
// constraint, so `a[v] = 3` with `v: T extends number` evolves `a`.
enum Foo {
	A,
	B,
}
export function f<T extends 0 | 1 | 2 | number>(v: T) {
	let a = [];
	a[v] = 3;
	return a;
}
export function g<T extends Foo>(v: T) {
	const a = [];
	a[v] = 3;
	return a;
}
