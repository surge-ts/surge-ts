// `M extends Exactly<Mask, M>` names the parameter in its own constraint: the
// candidate is checked against the alias instantiated with itself, and an
// inferred argument that fails it is checked against that constraint.
type Mask<K extends PropertyKey> = { [k in K]?: true };
type Exactly<T, X> = T & Record<Exclude<keyof X, keyof T>, never>;
declare class Obj<S> {
    pick<M extends Exactly<Mask<keyof S>, M>>(mask: M): void;
}
declare const fish: Obj<{ name: string; age: number }>;
fish.pick({ name: true, age: false });
fish.pick({ name: true, extra: true });
fish.pick({ name: true });
declare function only<M extends Exactly<Mask<"a" | "b">, M>>(m: M): void;
only({ a: true, c: true });
only({ a: false });
only({ b: true });
