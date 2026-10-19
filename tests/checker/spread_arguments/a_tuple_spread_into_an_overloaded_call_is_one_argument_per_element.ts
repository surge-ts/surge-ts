// @surge-compare: messages
// A spread of a tuple stands for its elements (`getEffectiveCallArguments`),
// so the overload is chosen by the arguments they make.
declare function pick(a: string, ...rest: number[]): string;
declare function pick(a: number, ...rest: string[]): number;
declare function either(a: string, b?: number): string;
declare function either(a: number): number;
const pair: [string, number] = ["a", 1];
const head: [string, number?] = ["a"];
const one: [number] = [1];
const strings: string[] = ["a"];
const a: boolean = pick(...pair);
const b: boolean = either(...head);
const c: boolean = pick(...one, ...strings);
const d: boolean = pick(...[1], ...strings);
export {};
