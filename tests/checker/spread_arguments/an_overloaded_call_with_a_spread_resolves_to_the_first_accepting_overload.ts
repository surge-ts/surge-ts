// @surge-compare: messages
// `chooseOverload` judges a spread call's arity by where the spread begins and
// relates what it iterates to the rest, so the call is its first accepting
// overload's, as it is for one that is not overloaded.
declare function select<T>(index: number, ...args: Array<T>): [T];
declare function select(cmd: "#", ...args: Array<unknown>): number;
declare function one(index: number, ...args: Array<unknown>): [unknown];
declare function two(index: number, ...args: Array<unknown>): [unknown];
declare function two(cmd: "#", ...args: Array<unknown>): number;
const args: unknown[] = ["123", "456"];
const a: boolean = select(1, ...args);
const b: boolean = one(1, ...args);
const c: boolean = two(1, ...args);
const d: boolean = two("#", ...args);
const strings: string[] = ["a"];
declare function pick(a: string, ...rest: number[]): string;
declare function pick(a: number, ...rest: string[]): number;
const e: boolean = pick(1, ...strings);
declare function optional(a: number, b?: string): string;
declare function optional(a: string): number;
const f: boolean = optional(1, ...strings);
export {};
