// @strict: true
// @lib: es2023
interface User { id: number; name: string; tags?: string[] }
declare const users: User[];
declare function idf<T>(x: T): T;
const nested: string = idf(users.map(u => u.id));
const inObject: string = { v: users.map(u => u.id) };
const inArray: string = [users.map(u => u.id)];
const flat: number[] = users.flatMap(u => u.tags ?? []);
const flatNested: string = idf(users.flatMap(u => [u.id]));
const callable: string[] = "1,2".split(",").map(Number);
const ids: string = new Set(users.map(u => u.id));
const literal: string = new Set([1, 2]);
const kept: ReadonlySet<"a" | "b" | "c"> = new Set(["a", "b"]);
declare function maybeArray<T>(values: readonly T[] | null): T;
const fromUnion: string = maybeArray(users);
