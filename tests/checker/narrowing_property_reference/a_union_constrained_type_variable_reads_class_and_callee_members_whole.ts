class P { a() {} }
class Q { b() {} }
function f1<V extends P | Q>(v: V) { v.a; }
function f2<V extends P | Q>(v: V) { v.a(); }
class GP<T> { value!: T; a() {} }
class GQ<T> { value!: T; b() {} }
function f3<V extends GP<number> | GQ<string>>(v: V) { v.a(); }
type PO = { a(): void };
type QO = { b(): void };
function f4<V extends PO | QO>(v: V) { v.a; v.a(); }
function f5<V extends PO | QO>(v: V) { if ("a" in v) v.a(); else v.b(); }
function f6<T extends string | (new () => {})>(a: T) {
    if (typeof a !== "string") {
        new a();
    }
}
function f7<K extends 'a' | 'b'>(k: K) {
    if (k === 'a') { const z: 'a' = k; }
}
export {};
