// A write to a member only an index signature answers, on a named type that
// declares members of its own, leaves the type itself: it stays iterable.
export {};
interface Table extends Iterable<[string, number]> {
	readonly kind: "table";
	[key: string]: number | "table";
}
declare const TableConstructor: new () => Table;
const make = () => {
	const source = new TableConstructor();
	source.a = 1;
	const [first, ...rest] = source;
	const firstKey: number = first[0];
	const restCount: string = rest.length;
	for (const [key, entry] of source) {
		const keyText: number = key;
	}
};
