// @strict: true
// `ReadonlyArray`'s generic methods infer their type parameters from the
// callback as `Array`'s do.
interface Item { readonly id: string; readonly source: string }
export function f(xs: readonly Item[], ys: ReadonlyArray<string>, maybe?: readonly Item[]) {
  const ids: number[] = xs.map((x) => x.id);
  const lengths: string[] = ys.map((y) => y.length);
  const total: string = ys.reduce((sum, y) => sum + y.length, 0);
  const byId = new Map((maybe ?? []).map((x) => [x.id, x] as const));
  const found = byId.get("a");
  if (!found) return;
  const source: number = found.source;
  return [ids, lengths, total, source];
}
