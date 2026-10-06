// A type literal in an alias body is written in the alias's own parameters,
// which the reference's arguments replace before the argument is matched.
type Box1<T> = { value: T };
type Box2<T> = { value: T };

declare function foo<T>(x: Box1<Box1<T>>): T;
declare let z: Box2<Box2<string>>;
const s: string = foo(z);
const n: number = foo(z);

type Pair<A, B> = { first: A; second: { inner: B } };
declare function unpair<A, B>(p: Pair<B, A>): [A, B];
declare let p: { first: number; second: { inner: string } };
const swapped: [string, number] = unpair(p);
const wrong: [number, string] = unpair(p);
