// A generic object-literal method or property function whose signature is
// written out whole instantiates its type parameters at a call through the
// object, inferred or written.
const o = { wrap<T>(x: T): { v: T } { return { v: x }; } };
const inferred: number = o.wrap("a");
const written: number = o.wrap<string>("a");
const unannotated = o.wrap("a");
const read: number = unannotated;
const p = { wrap: <T,>(x: T): { v: T } => ({ v: x }) };
const property: number = p.wrap("a");
const q = { id<T>(x: unknown): T { return x as T; } };
const explicit: number = q.id<string>(1);
function outer<U>(u: U) {
    const r = { pair<T>(x: T): [T, U] { return [x, u]; } };
    const pair: number = r.pair("a");
}
