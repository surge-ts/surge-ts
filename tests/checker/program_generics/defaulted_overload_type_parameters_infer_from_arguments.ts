// @strict: true
// @lib: es2023
interface Item {
  id: string;
  price: number;
}
declare const items: Item[];
const byId = Object.fromEntries(items.map((item) => [item.id, item]));
const wrongById: string = byId["a"];
const pairs: [string, number][] = [["a", 1]];
const prices = Object.fromEntries(pairs);
const wrongPrices: string = prices["a"];
interface Entries {
  collect<T = any>(entries: Iterable<readonly [PropertyKey, T]>): { [key: string]: T };
  collect(entries: Iterable<readonly any[]>): any;
}
declare const entries: Entries;
const collected = entries.collect(pairs);
const wrongCollected: string = collected["a"];
