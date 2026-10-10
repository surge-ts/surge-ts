// @strict: true
// @lib: es2023
type Key = "a" | "b" | "c";
const kinds: ReadonlySet<Key> = new Set(["a", "b"]);
const prices: ReadonlyMap<Key, number> = new Map([["a", 1]]);
declare function takeKinds(kinds: ReadonlySet<Key>): void;
takeKinds(new Set(["a"]));
declare class Registry<K, V> {
  constructor(entries?: readonly (readonly [K, V])[] | null);
  key: K;
}
const registry: { key: Key } = new Registry([["a", 1]]);
const free = new Map([["a", 1]]);
free.set("z", 2);
const outside: ReadonlyMap<Key, number> = new Map([["z", 1]]);
