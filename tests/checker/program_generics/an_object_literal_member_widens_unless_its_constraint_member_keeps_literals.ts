// @strict: true
// A primitive in the constraint keeps a bare literal argument whole, but each
// member of an object literal argument is typed against the constraint's own
// member: it stays a literal only where that member's type is literal-ish.
type Anim = number | boolean | Record<string, number>;
interface Spring<T extends Anim> {
  setGoal(value: Partial<T>): void;
}
declare function createSpring<T extends Anim>(initial: T): Spring<T>;
createSpring({ x: 0, y: 0 }).setGoal({ x: 10 });
declare function keep<T extends Anim>(initial: T): T;
export const zero: 0 = keep(0);
declare function tagged<T extends number | { kind: "a" | "b"; n: number; on: boolean }>(v: T): T;
const t = tagged({ kind: "a", n: 1, on: true });
t.n = 2;
export const kind: "a" = t.kind;
export const on: true = t.on;
declare function loose<T extends string | { k: unknown }>(v: T): T;
const l = loose({ k: "a" });
l.k = "b";
declare function nested<T extends string | { inner: { v: number } }>(v: T): T;
const n = nested({ inner: { v: 1 } });
n.inner.v = 2;
