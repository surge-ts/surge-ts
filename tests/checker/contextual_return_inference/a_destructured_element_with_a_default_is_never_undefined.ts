// @surge-compare: messages
// An array binding element with a default binds it for a missing element, so
// neither the name nor a pattern nested in it reads `undefined`.
function plain([a = 1]: [number?]) {
	const read: string = a;
}
function rest(...[a = 1]: [number?]) {
	const read: string = a;
}
function nested([{ v } = { v: 1 }]: [{ v: number }?]) {
	const read: string = v;
}
function restNested(...[{ v } = { v: 1 }]: [{ v: number }?]) {
	const read: string = v;
}
export {};
