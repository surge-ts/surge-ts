// @strict: true
// @lib: es2023
const byName = new Map([["a", 1], ["b", 2]]);
const wrongMap: number = byName;
const found: string = byName.get("a");
const numeric = new Map([[1, 2]]);
const wrongKey: string = [...numeric.keys()][0];
declare class Registry<K, V> {
  constructor(entries?: readonly (readonly [K, V])[] | null);
  key: K;
  value: V;
}
const registry = new Registry([["x", true]]);
const wrongValue: number = registry.value;
declare function fromPairs<K, V>(entries: readonly (readonly [K, V])[]): Map<K, V>;
const widened = fromPairs([["a", 1]]);
const wrongWidened: Map<"a", 1> = widened;
