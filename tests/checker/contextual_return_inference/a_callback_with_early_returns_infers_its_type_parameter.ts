// @strict: true
// A block-bodied callback with early returns infers the type parameter its
// return names from every `return`, each read under the narrowing that
// reaches it, with `undefined` for an end it can reach.
declare const flag: boolean;
declare function plain<V>(f: () => V): V;
declare function each<V>(f: (x: string | null) => V | Promise<V>): V;

export const a: string = plain(() => { if (flag) return 1; return 2; });
export const b: string = each((x) => { if (!x) return null; return x.length; });
export const c: number = each((x) => { if (typeof x === "string") return x; else return 0; });
export const d: number = plain(() => { if (flag) { return 1; } });
export const e: { k: number } = each((x) => { if (x) { return { k: x }; } return { k: "none" }; });
