// @strict: true
// A class a namespace declares, in a source or a `declare` namespace,
// generic or not, is its constructor value: `new` builds its instance and
// its statics are typed.
export namespace N { export class C { x = 1; static s = "a"; } export class G<T> { v!: T } }
declare namespace D { class E { e: number; static t: string } }
const a: string = new N.C();
const b: number = N.C;
const c: number = N.C.s;
const d: string = new D.E();
const e: number = D.E.t;
const f: number = new N.G<string>();
const g: number = N.G;
export {};
