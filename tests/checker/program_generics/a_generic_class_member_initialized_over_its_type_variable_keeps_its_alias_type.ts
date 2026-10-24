// An unannotated member whose initializer instantiates an alias over the
// class's type variable is that alias instantiation, however it is first read:
// from the module's own top-level statements, through `this`, or through an
// instance over another body's type variable.
interface Bag<T> { x: T }
type Box<T> = Bag<T>;
declare function make<T>(): Box<T>;
export class C<T> {
	a = make<(value: T) => void>();
	b: Box<(value: T) => void> = make<(value: T) => void>();
	tuple = make<[T]>();
	list = make<T[]>();
	record = make<{ v: T }>();
	m() {
		const n: number = this.a;
		const o: number = this.b;
		const x: number = this.a.x;
		const t: number = this.tuple;
		const l: number = this.list;
		const r: number = this.record;
	}
}
declare const c: C<string>;
const p: number = c.a;
const q: number = c.a.x;
function read<V>(d: C<V>) {
	const n: number = d.a;
	const x: number = d.list.x;
}
