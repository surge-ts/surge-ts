type Writable<T> = { -readonly [P in keyof T]: T[P] };
type Id<T> = { [P in keyof T]: T[P] };
const a1: 1 = null! as Writable<readonly [string, ...string[]]>;
const a2: 1 = null! as Writable<[string, ...string[]]>;
const a3: 1 = null! as Writable<readonly [string, number]>;
const a4: 1 = null! as Id<[string, ...string[]]>;
const a5: 1 = null! as Writable<readonly string[]>;
const a6: 1 = null! as Readonly<[string, ...string[]]>;
