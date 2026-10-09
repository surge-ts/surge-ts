// @strict: true
// @lib: es2023
class Ring<T> {
  list: T[] = [];
  value!: T;
  first() { return this.list[0]; }
  current() { return this.value; }
  wrap() { return { items: this.list }; }
  pair(x: T) { return [x, x] as const; }
  firstOf(xs: T[]) { return xs[0]; }
  get size() { return this.list.length; }
}
declare const ring: Ring<boolean>;
const first: number = ring.first();
const current: number = ring.current();
const wrapped: number = ring.wrap();
const paired: number = ring.pair(true);
const firstOf: number = ring.firstOf([true]);
const size: string = ring.size;
class Words extends Ring<string> {}
const word: number = new Words().first();
const kept: string | undefined = new Words().first();
