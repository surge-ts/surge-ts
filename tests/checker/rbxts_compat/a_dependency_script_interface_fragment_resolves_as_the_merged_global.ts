// @types: tbl
// A global interface a dependency's script declarations split across files
// is one merged declaration: the constructor the first file declares returns
// the interface the second reopens with `extends Iterable`, wherever the
// first read of it happens (here inside a callback argument).
// @filename: node_modules/@types/tbl/package.json
{ "name": "@types/tbl", "version": "1.0.0", "types": "index.d.ts" }
// @filename: node_modules/@types/tbl/index.d.ts
/// <reference path="iterable.d.ts" />
interface Tbl {
	readonly kind: number;
	[key: string]: number;
}
interface TblConstructor {
	new (): Tbl;
}
declare const Tbl: TblConstructor;
// @filename: node_modules/@types/tbl/iterable.d.ts
interface Tbl extends Iterable<[string, number]> {}
// @filename: index.ts
declare function run(f: () => void): void;
run(() => {
	const table = new Tbl();
	const [first, ...rest] = table;
	const text: string = first;
	const count: string = rest;
	for (const entry of table) {
		const entryText: string = entry;
	}
});
export {};
