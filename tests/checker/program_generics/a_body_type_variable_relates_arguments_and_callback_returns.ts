// @strict: true
// The type variables of the body being checked are types like any other: a
// `T` passed where an `S extends T` is expected is not an `S`, whether as an
// argument, as a callback's return, or as the candidate an inference would
// fix for a type parameter constrained to `S` (which then falls back to `S`).
function args<T, S extends T>(t: T, s: S, f: (v: S) => void, o: { n(v: S): void }) {
    f(t);
    o.n(t);
    f(s);
}

declare function pair<A>(a: A, b: A): void;
function explicit<T, S extends T>(t: T, s: S) {
    pair<S>(s, t);
    pair<T>(t, s);
}

class Chain<T> {
    constructor(public value: T) { }
    m(cb: (x: T) => T): void {}
    then<S extends T>(cb: (x: T) => S): Chain<S> { return null!; }
}
function callbacks<T, S extends T>(t: T, s: S, c: Chain<S>) {
    c.m(ss => t);
    c.m(ss => s);
    c.m((ss) => { return t; });
    new Chain(s).then(ss => t);
    new Chain(t).then(tt => s).then(ss => t);
    const annotated: (x: S) => S = ss => t;
}

function identity<T>(x: T): T { return x; }
function widened<T, U extends T>(u: U): T { return u; }
function mapped<T>(xs: T[]): T[] { return xs.map(x => x); }
