// @strict: true
type Writable<T> = { -readonly [P in keyof T]: T[P] };
type Config<TEnum extends readonly string[] | string[] | undefined> = { enum?: TEnum };
interface Builder<T> { _: { enumValues: T } }
declare function text(): Builder<[string, ...string[]]>;
declare function text<U extends string, T extends Readonly<[U, ...U[]]>>(name: string, config?: Config<T | Writable<T>>): Builder<Writable<T>>;
declare function table<T extends Record<string, Builder<any>>>(name: string, cols: T): { cols: T };

const columns = table('t', { klass: text('c', { enum: ['A', 'C'] }), plain: text() });
export const klass: ['A', 'C'] = columns.cols.klass._.enumValues;
export const wrong: ['A', 'B'] = columns.cols.klass._.enumValues;
export const plain: [string, ...string[]] = columns.cols.plain._.enumValues;
