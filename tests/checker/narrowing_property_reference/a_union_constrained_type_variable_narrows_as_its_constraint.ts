type A = { tag: 'A'; foo: number };
type B = { tag: 'B'; bar: string };
type MyUnion = A | B;
function f1<T extends MyUnion>(value: T) {
    value.foo;
    if ('foo' in value) value.foo;
    if (value.tag === 'A') value.foo;
    switch (value.tag) { case 'A': value.foo; break; }
    value.tag;
}
function f2<T extends string | undefined>(x: T) {
    x.length;
    if (x) x.length;
    if (x !== undefined) x.length;
    if (typeof x === 'string') x.length;
}
function f3<T extends A | undefined>(x: T) {
    x.foo;
    x?.foo;
    if (x) x.foo;
}
function f4<T extends number[] | string>(x: T) {
    x.length;
    x.map;
    if (Array.isArray(x)) x.map(n => n);
    if (typeof x !== 'string') x.map(n => n);
}
function f5<T extends MyUnion>(value: T) {
    if (value.tag !== 'A') return;
    value.foo;
}
function f6<T extends A | B>(value: T, other: T) {
    if ('foo' in value) other.foo;
}
type Action2 = { kind: 'A', payload: number } | { kind: 'B', payload: string };
function g13(t: Action2) {
    const { kind, payload } = t;
    if (kind === 'A') { payload.toFixed(); }
}
function g14<T extends Action2>(t: T) {
    const { kind, payload } = t;
    if (kind === 'A') { payload.toFixed(); }
}
function g15<T extends Action2>(t: T) {
    if (t.kind === 'A') { t.payload.toFixed(); }
}
function g16<T extends Action2>(t: T) {
    const kind = t.kind;
    if (kind === 'A') { t.payload.toFixed(); }
}
export {};
