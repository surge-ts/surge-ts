// A target nested under a rest element of a destructuring assignment reads
// the source's tail (`sliceTupleType`), through numeric names and `${number}`
// keys too; the write ends the narrowing before it.
export {};
declare function assert(value: unknown): asserts value;
declare const source: [number, ...number[][]];
declare const arrays: number[][];
declare const key: `${number}`;
export function f() {
	let first = 0;
	let value = 0;
	assert(value === 42);
	[first, ...[[value]]] = source;
	assert(value === 99);
	assert(value === 42);
	let s1 = "";
	[first, ...[[s1]]] = source;
	let s2 = "";
	[first, ...[s2]] = source;
	let s3 = "";
	[...[{ 1: [s3] }]] = [source];
	let s4 = "";
	({ 1: [s4] } = source);
	let s5 = "";
	[...{ [key]: [s5] }] = arrays;
	const { [key]: s6 } = arrays;
	const s7: string = s6;
}
