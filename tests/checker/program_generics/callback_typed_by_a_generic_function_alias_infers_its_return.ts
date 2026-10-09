// @strict: true
// @surge-compare: messages
interface Dictionary<T> { [index: string]: T }
interface Foo { foo: string }
type ObjectIterator<TObject, TResult> = (value: TObject[keyof TObject], key: string, collection: TObject) => TResult;
declare const foos: Record<string, Foo>;
declare function mapValues<T, R>(obj: Dictionary<T> | null, cb: ObjectIterator<Dictionary<T>, R>): Dictionary<R>;
export const read: 0 = mapValues(foos, (f) => f.foo.length);
