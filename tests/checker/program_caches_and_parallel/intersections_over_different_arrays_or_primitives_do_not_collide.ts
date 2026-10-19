// @surge-compare: messages
// An intersection is its operands: `[string] & Tag` and `string[] & Tag`
// merge to the same members, as do `number & Tag` and `string & Tag`, yet each
// instantiation over one must not be served for the other, and a union keeps
// both.
export {};
interface Tag {
  tag: number;
}
interface Box<T> {
  value: T;
}
declare const list: Box<string[] & Tag>;
declare const single: Box<[string] & Tag>;
declare const pair: Box<[string, string] & Tag>;
const one: [string] = single.value;
const two: [string, string] = pair.value;
const notOne: boolean = pair.value;

declare const id: Box<number & Tag>;
declare const name: Box<string & Tag>;
const label: string = name.value;
const numeric: number = name.value;

declare const either: (string[] & Tag) | ([string] & Tag);
const unrelated: boolean = either;
