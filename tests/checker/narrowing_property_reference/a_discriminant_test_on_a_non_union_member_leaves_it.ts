// @strict: true
// A literal test of a member whose declared type is no union narrows only the
// member read, not the object holding it (`isMatchingReferenceDiscriminant`).
interface Rec { readonly generated: { readonly flag: true; readonly name: string }; readonly owner: "server" }
export function f(previous: Rec, current: Rec) {
  if (previous.generated.flag !== true) {
    return previous.generated.name;
  }
  if (previous.generated.flag !== current.generated.flag) {
    return `${previous.generated.flag} ${previous.generated.name}`;
  }
  const changed = previous.owner !== current.owner;
  return changed && previous.owner === "server" && current.owner !== "server";
}
