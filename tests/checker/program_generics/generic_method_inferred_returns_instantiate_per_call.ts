// @strict: true
// @lib: es2023
class Box<T> { constructor(public value: T) {} }
class Maker {
  wrap<T>(x: T) { return { value: x }; }
  id<T>(x: T) { return x; }
  pair<A, B>(a: A, b: B) { return [a, b] as const; }
  list<T>(...xs: T[]) { return xs; }
  boxed<T>(x: T) { return new Box(x); }
}
const maker = new Maker();
const wrapped: number = maker.wrap("a");
const same: number = maker.id("s");
const paired: number = maker.pair(1, "a");
const listed: number = maker.list(1, 2);
const boxedValue: string = maker.boxed(1).value;
class Mapper<S> {
  map<R>(f: (s: S) => R) { return new Mapper<R>(); }
  read<K extends keyof S>(source: S, k: K) { return source[k]; }
}
const mapper = new Mapper<{ a: number }>();
const mapped: number = mapper.map(s => String(s.a));
const read: string = mapper.read({ a: 1 }, "a");
class Store<S> {
  static create<S>(initial: S) { return new Store(initial); }
  static of<T>(value: T) { return new Box(value); }
  constructor(private state: S) {}
  select<R>(fn: (s: S) => R): R { return fn(this.state); }
}
const selected: string = Store.create({ count: 1, name: "x" }).select(s => s.count);
const boxedStatic: string = Store.of(1).value;
