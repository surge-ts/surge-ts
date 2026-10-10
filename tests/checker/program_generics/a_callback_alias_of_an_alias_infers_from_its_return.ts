// @strict: true
// An alias of a callback alias stands for the aliased signature, so a
// context-sensitive callback's return still infers through it.
interface Dict<T> { [index: string]: T; }
interface Foo { foo: string }
declare const foos: Record<string, Foo>;
type OI<TObject, TResult> = (value: TObject[keyof TObject], key: string, collection: TObject) => TResult;
type DI<T, R> = OI<Dict<T>, R>;
declare function mv<T, R>(obj: Dict<T>, cb: DI<T, R>): Dict<R>;
const r = mv(foos, f => f.foo);
const n: Dict<number> = r;
const s: Dict<string> = r;
