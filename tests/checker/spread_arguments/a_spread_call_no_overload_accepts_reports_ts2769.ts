// @surge-compare: messages
// With every candidate whose arity fits a spread call rejecting it, the call
// is TS2769 with the last candidate's first rejected argument: a spread for
// what it iterates or for each tuple element it stands for. A lone fitting
// candidate reports its own error.
declare function pick(a: string, ...rest: number[]): string;
declare function pick(a: number, ...rest: string[]): number;
const numbers: number[] = [1];
const strings: string[] = ["a"];
const pair: [number, number] = [1, 2];
pick(1, ...numbers);
pick(true, ...numbers);
pick(...pair);
pick(1, ...[2, 3]);
pick(1, ...numbers, "x");
declare function lone(a: string, ...rest: number[]): string;
declare function lone(a: string, b: string, c: string): number;
lone("a", ...strings);
// `reorderCandidates` moves an overload with a literal-typed parameter first,
// so the last one re-checked is the last of the others.
declare function three(a: string, ...rest: number[]): string;
declare function three(a: number, ...rest: string[]): number;
declare function three(a: boolean, ...rest: boolean[]): boolean;
declare function three(a: null, ...rest: null[]): null;
three(1, ...numbers);
interface Store {
  get(key: string, ...rest: number[]): string;
  get(key: number, ...rest: string[]): number;
}
declare const store: Store;
store.get(1, ...numbers);
export {};
