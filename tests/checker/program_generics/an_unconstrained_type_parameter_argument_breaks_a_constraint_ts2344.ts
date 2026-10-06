export type Foo<T extends string> = { foo: T };
function f<T>(x: Foo<T>) { const y: Foo<T> = x; return y; }
interface I<T> { a: Foo<T> }
type Bar<T> = Foo<T>;
type Ok<T extends string> = Foo<T>;
export { f };
export type { I, Bar, Ok };
