// A string destructured by a `for…of` head binds the strings it iterates: an
// element is a `string`, a rest element a `string[]`.
for (const [first, ...rest] of ["abc", "def"]) {
	const f: boolean = first;
	const r: boolean = rest;
}
function read(text: string) {
	for (const [head, ...tail] of [text]) {
		const h: boolean = head;
		const t: boolean = tail;
	}
}
