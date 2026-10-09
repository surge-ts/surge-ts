// @strict: true
// @lib: es2023
class Base {
  items: boolean[] = [];
  self() { return this; }
  chained(flag: boolean) {
    if (flag) {
      return this;
    }
    return this;
  }
  other() { return new Base(); }
  maybe(flag: boolean) { if (flag) return this; }
}
class Derived extends Base {
  extra = 1;
}
const base = new Base();
const fromSelf: number = base.self();
const fromOther: number = base.other();
let annotated: Derived;
const derived = new Derived();
const keepsDerived: Derived = derived.self().chained(true);
const derivedSelf: number = derived.self();
const maybeUndefined: Base = base.maybe(true);
class Box<T> {
  value!: T;
  set(value: T) { this.value = value; return this; }
}
const box: number = new Box<string>().set("a");
class Counter {
  self = this;
  next() { return { value: 1, done: false }; }
  [Symbol.iterator]() { return this; }
}
class Labeled extends Counter {
  label = "";
}
const labeled = new Labeled();
const viaProperty: Labeled = labeled.self;
const viaPropertyWrong: number = labeled.self;
for (const value of new Counter()) {
  const text: string = value;
}
