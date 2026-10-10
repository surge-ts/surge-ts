// @strict: true
// @lib: es2023
interface EachReturn<T extends any[]> {
  (name: string, fn: (...args: T) => void): void;
}
interface Each {
  <T extends any[] | [any]>(cases: ReadonlyArray<T>): EachReturn<T>;
  <T>(cases: ReadonlyArray<T>): EachReturn<T[]>;
  (...args: [TemplateStringsArray, ...any]): EachReturn<any[]>;
}
declare const each: Each;
each(["http", "ws"] as const)("protocol %s", (protocol) => {});
interface Pick2 {
  (cases: ReadonlyArray<number[]>): 1;
  <T>(cases: ReadonlyArray<T>): T;
}
declare const pick2: Pick2;
const picked: number = pick2(["a", "b"] as const);
