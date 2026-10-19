// @surge-compare: messages
// A method group and a binding inferred from the call read the same overload
// as the checked call, a LuaTuple-returning one indexed by position.
type LuaTuple<T extends Array<any>> = T & { readonly _nominal_LuaTuple: unique symbol };
declare function select(index: number, ...args: Array<unknown>): LuaTuple<Array<unknown>>;
declare function select(cmd: "#", ...args: Array<unknown>): number;
interface Store {
  get(key: string, ...rest: number[]): string;
  get(key: number, ...rest: string[]): number;
}
declare const store: Store;
const args: unknown[] = ["123", "456"];
const strings: readonly string[] = ["a"];
const tuple = select(1, ...args);
const a: boolean = tuple;
const b: boolean = select(1, ...args)[1];
const c: boolean = select("#", ...args);
const value = store.get(1, ...strings);
const d: boolean = value;
const e: boolean = store.get("k", ...[1, 2]);
export {};
