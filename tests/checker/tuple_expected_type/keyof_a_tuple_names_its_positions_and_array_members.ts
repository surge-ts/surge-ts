type Pair = [{ name: "x" }, { name: "y" }];
const position: keyof Pair = "1";
const outOfRange: keyof Pair = "2";
const member: keyof Pair = "map";
const index: keyof Pair = 5;
const pushed: keyof readonly string[] = "push";
const optional: keyof [a: number, b?: string, ...rest: boolean[]] = "2";

type IndexFor<S extends string> = S extends `${infer N extends number}` ? N : never;
type IndicesOf<T> = IndexFor<Extract<keyof T, string>>;
declare function at<TDef extends readonly unknown[], I extends IndicesOf<TDef>>(definition: TDef, index: I): void;
declare const pair: Pair;
at(pair, 1);
at(pair, 2);
