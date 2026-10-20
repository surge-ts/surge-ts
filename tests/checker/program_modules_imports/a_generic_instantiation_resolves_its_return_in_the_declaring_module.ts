// `createFor`'s `Caller<T>` and the `Record1<T>` its body names are the
// declaring module's, not the caller's own `Record1`.
// @filename: lib.ts
export type Record1<T> = { [K in keyof T]: T[K] };
export type Caller<T> = (ctx: number) => Record1<T>;
export interface Factory {
    createFor<T>(value: T): Caller<T>;
}
export declare function createFor<T>(value: T): Caller<T>;
// @filename: consumer.ts
import type { Factory } from "./lib";
import { createFor } from "./lib";
export type Record1<A, B> = { a: A; b: B };
declare const factory: Factory;
export const viaMethod: number = factory.createFor({ name: "x" })(2).name;
export const viaFunction: number = createFor({ name: "x" })(2).name;
