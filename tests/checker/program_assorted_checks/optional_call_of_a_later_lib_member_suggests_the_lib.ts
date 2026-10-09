// @lib: es2020
// @strict: true
declare const items: { id: string }[];
const g = Object.groupBy?.(items, i => i.id);
const s = "abc".replaceAll?.("a", "b");
