class Box<T> {
    constructor(readonly v: T) {}
    m(this: Box<string>) { return 1; }
    g<U>(this: Box<string>, u: U) { return u; }
}
declare const b: Box<number>;
b.m();
b.g(1);
interface I { f(this: { x: number }): void; }
declare const i: I;
i.f();
interface J { f(this: { x: number, z: string }): void; }
declare const j: J;
j.f();
class EPromise<E, A> implements PromiseLike<A> {
    constructor(readonly e: E, readonly a: A) {}
    then<B = A, B1 = never>(
        this: EPromise<never, A>,
        onfulfilled?: ((value: A) => B | PromiseLike<B>) | null,
        onrejected?: ((reason: any) => B1 | PromiseLike<B1>) | null
    ): PromiseLike<B | B1> {
        return null!;
    }
}
declare const failing: EPromise<number, string>;
failing.then(s => s.length);
async function run() {
    await failing;
}
declare const passing: EPromise<never, string>;
passing.then(s => s.length);
async function run2() {
    await passing;
}
export {};
