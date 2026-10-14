// @strict: true
// A primitive meets `{ valueOf(): R }` only where it is an `R`, so a generic
// overload whose constraint it breaks gives way to the next one (Node's
// `Buffer.from(string)` past `from<T extends WithImplicitCoercion<ArrayBufferLike>>`).
type WithImplicitCoercion<T> = T | { valueOf(): T };
interface Buf<T extends ArrayBufferLike = ArrayBufferLike> { readonly buffer: T }
interface BufCtor {
  from<T extends WithImplicitCoercion<ArrayBufferLike>>(
    arrayBuffer: T,
  ): Buf<T extends { valueOf(): infer V extends ArrayBufferLike } ? V : T>;
  from(text: WithImplicitCoercion<string>, encoding?: string): Buf<ArrayBuffer>;
}
declare const B: BufCtor;
const entries = [{ name: "x", data: B.from("1") }];
export const one: Buf = entries[0]!.data;
export const two: Buf<ArrayBuffer> = B.from("2");
