// `inferFromTypes` records an `any` source for a naked type parameter only:
// an `any` member of an object literal argument infers nothing through the
// structured parameter it meets, and the other members decide.
interface Schema<T> {
    _t: T;
}
declare function use<T>(p: { schema: Schema<T>; def: Partial<T> }): { reg(k: keyof T): void };
declare const missing: Missing;
declare const anything: any;
use({ schema: missing, def: { name: "x" } }).reg("image");
use({ schema: anything, def: { name: "x" } }).reg("image");
declare function one<T>(x: Schema<T>, y: T): T;
const r: string = one(anything, 1);
declare function naked<T>(x: T, y: T): T;
const n: string = naked(anything, 1);
declare function partial<T>(def: Partial<T>): { reg(k: keyof T): void };
partial({ name: "x" }).reg("image");
