// @strict: true
export {}
interface Hm { a: { x: number }; b: { y: string } }
interface Em { [index: number]: { z: boolean } }
export function f1<T extends number, V extends Hm[T]>(t: T, v: V) {}
export function f2<T extends keyof Em>(x: Hm[T]) {}
export function f3<T extends "a" | "c">(x: Hm[T]) {}
export function f4<T extends keyof Em, V extends Hm[T]>() {}
type K = keyof Em;
export function f5<T extends K>(x: Hm[T]) {}
declare global {
    interface Gm { [index: number]: { z: boolean } }
}
export function f6<T extends keyof Gm, V extends Hm[T]>(t: T, v: V) {}
interface Nm { [index: number]: string; 0: string }
export function f7<T extends number>(x: Nm[T]) {}
export function f8<T extends number>(x: string[][T]) {}
type Wide = Hm & Pick<{ c: 1; a: 2 }, "c">;
export function f9<T extends keyof Wide>(x: Hm[T]) {}
export function f10<T extends "a" | 1>(x: Hm[T]) {}
export function f11<T extends string>(x: Hm[T]) {}
