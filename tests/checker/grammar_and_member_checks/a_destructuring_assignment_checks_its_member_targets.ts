// A destructuring assignment checks a member target as it checks the
// assignment of what the pattern reads for it (`checkDestructuringAssignment`):
// each element or property against the member, a readonly or missing member
// reported on its name.
interface Target {
	n: number;
	s: string;
	readonly fixed: number;
	optional?: boolean;
	list: number[];
}
declare const t: Target;
declare const pair: [number, string];
declare const strings: string[];
declare const record: Record<string, number>;
declare function identity<T>(value: T): T;
[t.n, t.s] = pair;
[t.s, t.n] = pair;
[t.fixed] = [1];
[t.missing] = [1];
[t["n"], t["s"]] = pair;
[t.n = "x"] = [undefined];
[t.optional] = [true];
[t.list[0], t.list[1]] = strings;
({ a: t.n, b: t.s } = { a: 1, b: 2 });
({ n: t.s } = t);
[record.anything, record["other"]] = [1, "two"];
[[t.n], { x: t.s }] = [[1], { x: 2 }];
[t.n, t.s] = identity<[string, string]>(["a", "b"]);
class C {
	#secret = 0;
	value = "";
	swap(other: C) {
		[this.#secret, this.value] = [1, 2];
		[other.value] = [3];
	}
}
export {};
