// @strict: true
// @lib: es2022
declare const entries: [string, string | undefined][];
declare function id<T>(value: T): T;
export const pairs: 0 = id(entries.filter((entry): entry is [string, string] => entry[1] !== undefined));
declare const headers: Record<string, string | undefined>;
export const normalized: Record<string, string> = Object.fromEntries(
    Object.entries(headers).filter((entry): entry is [string, string] => entry[1] !== undefined),
);
