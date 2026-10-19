// A `for…of` head destructuring an element that intersects a tuple (or an
// array) reads it by position through that operand, as roblox-ts's
// `LuaTuple<[A, B]>` elements are.
declare const pairs: Array<[string, number] & { tag: 1 }>;
for (const [key, value] of pairs) {
	const k: boolean = key;
	const v: boolean = value;
}
declare const rows: Array<string[] & { tag: 1 }>;
function read() {
	for (const [first, ...rest] of rows) {
		const f: boolean = first;
		const r: boolean = rest;
	}
}
