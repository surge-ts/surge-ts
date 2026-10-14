// @strict: true
// @noUncheckedIndexedAccess: true
// `unknown` past `=== null` is `{} | undefined`; a predicate to an
// index-signature type narrows `{}` to that type, which `{}` is no strict
// subtype of.
function isRecordLike(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null;
}
export function parse(value: unknown) {
  if (value === null) return null;
  if (!isRecordLike(value)) throw new Error("x");
  return typeof value["key"] === "string";
}
export function parse2(value: unknown) {
  if (value === undefined) return null;
  if (!isRecordLike(value)) throw new Error("x");
  return value["key"];
}
