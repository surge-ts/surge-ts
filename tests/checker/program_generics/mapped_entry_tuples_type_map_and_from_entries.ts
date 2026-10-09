// @strict: true
// @lib: es2023
interface Item { id: number; name: string }
declare const items: Item[];
const inferred = new Map(items.map(i => [i.id, i]));
const inferredRead: number = inferred;
const written = new Map<number, Item>(items.map(i => [i.id, i]));
const writtenRead: number = written;
const annotated: Map<number, Item> = new Map(items.map(i => [i.id, i]));
const entries = Object.fromEntries(items.map(i => [i.name, i.id]));
const entriesRead: number = entries;
const pairs: [string, number][] = items.map(i => [i.name, i.id]);
