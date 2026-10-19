// @noImplicitAny: true
// `isApplicableIndexType` against `String`'s number index admits a key of a
// numeric name (`"0"`, `"NaN"`) and `${number}`, as it does a `number`.
export {};
function getIndex(): "0" | "1" {
	return "1";
}
function read(index: "0" | "NaN") {
	return "abc"[index];
}
const a: string = "abc"[getIndex()];
function readTemplate(index: `${number}`) {
	return "abcdefghijk"[index];
}
function readEither(index: number | `${number}`) {
	return "abc"[index];
}
function readNonNumeric(index: "a" | "0") {
	return "abc"[index];
}
const viaRead: number = read("0");
