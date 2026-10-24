// An unannotated generic function returning an alias instantiated over its own
// type variable returns that alias over each call's type arguments, a caller's
// type variable among them.
interface Bag<T> { x: T }
type Box<T> = Bag<T>;
declare function make<U>(): Box<U>;
function callback<T>() { return make<(value: T) => void>(); }
function list<T>() { return make<T[]>(); }
function read<V>() {
	const a: number = callback<V>();
	const b: number = callback<V>().x;
	const c: number = list<V>();
}
const d: number = callback<string>();
const e: number = list<string>().x;
export {};
