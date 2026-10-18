// A function in an ambient namespace is its written signature, as a source
// namespace's is: its calls are checked and typed, an overloaded one by its
// overloads and a generic one by what its arguments infer (roblox-ts's
// `math.random()`, `vector.create()`).
// @filename: globals.d.ts
interface Point {
	readonly x: number;
}
declare namespace geometry {
	function make(x: number): Point;
	function random(): number;
	function random(max: number): string;
	function wrap<T>(value: T): { got: T };
	function isPoint(value: unknown): value is Point;
	const origin: Point;
}
interface vector {
	readonly z: number;
}
declare namespace vector {
	function create(z: number): vector;
}

// @filename: index.ts
export {};
const made: string = geometry.make(1);
geometry.make("one");
const none: string = geometry.random();
const some: number = geometry.random(5);
const wrapped: string = geometry.wrap(true).got;
declare const unknownValue: unknown;
if (geometry.isPoint(unknownValue)) {
	const narrowed: string = unknownValue.x;
}
const created: string = vector.create(1).z;
