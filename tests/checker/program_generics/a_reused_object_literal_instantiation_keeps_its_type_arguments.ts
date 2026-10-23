// @surge-compare: messages
interface Tag<T> { kind: "tag" }
type Unwrap<X> = X extends Tag<infer U> ? U : never;
type First = Unwrap<Tag<{ a: 1 }>>;
type Second = Unwrap<Tag<{ a: 1 }>>;
const first: First = { a: 2 };
const second: Second = { a: 2 };

declare function unwrap<U>(tag: Tag<U>): U;
declare const tagged: Tag<{ b: string }>;
declare const taggedAgain: Tag<{ b: string }>;
const fromCall: { b: number } = unwrap(tagged);
const fromCallAgain: { b: number } = unwrap(taggedAgain);

interface Box<T> { value: T }
declare const box: Box<{ c: number }>;
declare const boxAgain: Box<{ c: number }>;
const sameBox: Box<{ c: string }> = boxAgain;
