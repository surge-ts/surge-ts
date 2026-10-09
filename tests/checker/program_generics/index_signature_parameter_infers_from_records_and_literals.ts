// @strict: true
interface Dictionary<T> { [index: string]: T }
interface NumericDictionary<T> { [index: number]: T }
interface Foo { foo: string }
declare const record: Record<string, Foo>;
declare const literal: { a: Foo; b?: Foo };
declare const indexed: { [k: string]: Foo };
declare function viaInterface<T>(obj: Dictionary<T>): T;
declare function viaUnion<T>(obj: Dictionary<T> | NumericDictionary<T> | null): T;
declare function viaLiteral<T>(obj: { [k: string]: T }): T;
export const fromRecord: 0 = viaInterface(record);
export const fromLiteral: 0 = viaInterface(literal);
export const fromIndexed: 0 = viaInterface(indexed);
export const throughUnion: 0 = viaUnion(record);
export const typeLiteral: 0 = viaLiteral(record);
declare function mapValues<T, R>(obj: Dictionary<T> | null, cb: (value: T) => R): Dictionary<R>;
export const mapped: Record<string, number> = mapValues(record, (f) => f.foo);
