// @strict: true
// tsc tracks an unannotated `[]` declaration as an evolving array
// (`autoArrayType`) only when it is not exported: an exported one keeps its
// declared `never[]`, which other modules can see, so a push of anything is
// an error there.
export const exportedConst = [];
exportedConst.push(1);

export let exportedLet = [];
exportedLet.push("a");

const local = [];
local.push(1);
export const evolved: number[] = local;

export function inside(): number[] {
  const items = [];
  items.push(1);
  return items;
}
