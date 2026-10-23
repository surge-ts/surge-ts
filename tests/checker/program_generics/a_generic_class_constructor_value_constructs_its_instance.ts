// @strict: true
// A value typed by a generic class's constructor type (a parameter, a
// `const` copy, a declared one) constructs that class's instance, with its
// type arguments written or inferred, as `new` of the class itself does.
class B1<A = unknown> { v!: A; get(): A { return this.v; } }
class B2<A> { v!: A; constructor(v?: A) {} }
export function c1<A>(Ctor: typeof B1) { const n: number = new Ctor<A>(); }
export function c2<A>(Ctor: typeof B2) { const n: number = new Ctor<A>(); }
export function c3(Ctor: typeof B2) { const n: number = new Ctor<string>(); }
export function c4(Ctor: typeof B2) { const n: number = new Ctor(); }
export function c5(Ctor: typeof B2) { const n: number = new Ctor("s"); }
const K = B2;
export function c6<A>() { const n: number = new K<A>(); }
declare const ctor: typeof B2;
export const d3: number = new ctor<string>();
export function shadow(B2: typeof B1) { const n: number = new B2<string>(); }
abstract class Abstract<T> { v!: T }
export function abstractCtor(Ctor: typeof Abstract) { return new Ctor<string>(); }
