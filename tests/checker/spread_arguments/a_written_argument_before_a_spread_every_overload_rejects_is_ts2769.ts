// A written argument the fold rejects in a spread call is rejected by every
// candidate whose arity fits, so it is TS2769 rather than the fold's own error.
declare function shaped(o: { a: string }, ...rest: number[]): string;
declare function shaped(o: { b: number }, ...rest: string[]): number;
const numbers: number[] = [1];
const strings: string[] = ["a"];
shaped({ c: 1 }, ...numbers);
shaped({ a: "x" }, ...strings);
declare function pick(a: string, ...rest: number[]): string;
declare function pick(a: number, ...rest: string[]): number;
pick(true, ...numbers);
export {};
