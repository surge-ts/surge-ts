// A call that infers nothing for a type parameter fixes it at `unknown`; a
// return the body gives that does not name it is the call's return whatever
// it is fixed at, with or without a contextual type.
interface Box {
	value: number;
}
function boxed<T>() {
	const box: Box = { value: 1 };
	return box;
}
const contextual: string = boxed();
const free = boxed();
const read: string = free;
const fits: Box = boxed();
function outer() {
	function inner<T>() {
		return { value: 1, label: "inner" };
	}
	const nested: number = inner();
	const member: string = inner().value;
}
export {};
