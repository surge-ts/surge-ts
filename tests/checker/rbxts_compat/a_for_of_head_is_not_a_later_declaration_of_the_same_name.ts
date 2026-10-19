// A `for…of` head binds its own `const`: a same-named `let` declared later in
// the enclosing block is not what the body reads (no TS2448/TS2454).
declare const map: Map<string, number>;
export function pairs(values: number[]) {
	for (const pair of values) {
		console.log(pair);
	}
	let pair = 0;
	for (const [, value = 1] of map) {
		console.log(value);
	}
	let value = "";
	return [pair, value];
}
export function early() {
	console.log(later);
	let later = 0;
	return later;
}
