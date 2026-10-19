// @surge-compare: messages
// What a spread of a binding declared `unknown[]` iterates was written, so it
// infers the rest's type parameter as `unknown` like any declared argument.
declare function first<T>(index: number, ...args: Array<T>): [T];
const args: unknown[] = ["123", "456"];
const a: boolean = first(1, ...args);
export {};
