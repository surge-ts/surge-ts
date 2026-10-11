// @strict: true
// `A[T]` with `T extends keyof B` is checked where it is written
// (`checkIndexedAccessIndexType`): a key `B` has and `A` lacks makes `T` no
// valid index of `A` (TS2536).
interface A { a: 1 }
interface B { a: 1; b: 2 }
interface C { a: 1; b: 2; c: 3 }
function f<T extends keyof B>(x: A[T]) {}
type G<T extends keyof B> = A[T];
function g<T extends keyof B, V extends A[T]>(v: V) {}
type H<T extends keyof A> = A[T];
type I<T extends keyof B> = C[T];
