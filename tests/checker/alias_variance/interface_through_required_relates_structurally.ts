// @strict: true
interface Foo<T> {
  a: Required<T>;
}
const aa: Foo<{ a?: 1; x: 1 }> = { a: { a: 1, x: 1 } };
const bb: Foo<{ b?: 1; x: 1 }> = { a: { b: 1, x: 1 } };
export let AA = aa;
export let BB = bb;
AA = bb;
BB = aa;
