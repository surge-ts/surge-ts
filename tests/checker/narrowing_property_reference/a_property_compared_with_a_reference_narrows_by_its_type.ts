// @strict: true
// `o.p === value` narrows the property to its members comparable with what
// `value` is (tsc's `narrowTypeByEquality`), alone, through an optional
// chain, or as an operand of `&&`; `!==` leaves it whole.
type Thing = { foo: string | number };

export function compared(t: Thing, o: Thing | undefined, value: number, ok: boolean, either: string | number) {
  if (t.foo === value) {
    const a: number = t.foo;
  }
  if (o?.foo === value) {
    const b: number = o.foo;
  }
  if (o && o.foo === value) {
    const c: number = o.foo;
  }
  if (ok && t.foo === value) {
    const d: number = t.foo;
  }
  if (t.foo !== value) {
    const e: number = t.foo;
  }
  if (t.foo === either) {
    const f: number = t.foo;
  }
}
