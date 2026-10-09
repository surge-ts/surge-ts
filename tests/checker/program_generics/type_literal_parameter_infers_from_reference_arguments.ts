// @strict: true
interface Box<T> { value: T; tag: string }
declare const box: Box<number>;
declare function unbox<T>(x: { value: T }): T;
export const fromInterface: 0 = unbox(box);
class Holder<T> { constructor(public value: T) {} }
declare const holder: Holder<string>;
export const fromClass: 0 = unbox(holder);
type Pair<A> = { first: A; second: A };
declare const pair: Pair<boolean>;
declare function firstOf<T>(x: { first: T }): T;
export const fromAlias: 0 = firstOf(pair);
const inferred = unbox(box);
export const read: string = inferred;
