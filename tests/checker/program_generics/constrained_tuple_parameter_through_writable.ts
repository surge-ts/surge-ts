type Writable<T> = { -readonly [P in keyof T]: T[P] };
interface Builder<C extends { data: unknown; enumValues: unknown }> { _: C }
type TextInitial<TName extends string, TEnum extends [string, ...string[]]> = Builder<{ name: TName; data: TEnum[number]; enumValues: TEnum }>;
declare function text<TName extends string, U extends string, T extends Readonly<[U, ...U[]]>>(name: TName, config?: { enum?: T | Writable<T> }): TextInitial<TName, Writable<T>>;
const t = text('full');
const p: 1 = t._;
const w1: 1 = null! as Writable<readonly [string, ...string[]]>;
const w2: 1 = null! as Writable<readonly [string, ...number[]]>;
const w3: 1 = null! as Partial<[string, ...number[]]>;
