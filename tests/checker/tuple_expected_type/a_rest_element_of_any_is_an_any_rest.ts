// @strict: true
// @lib: es2023
type AnyRest = [any, ...any];
const display: number = null! as AnyRest;
type ReadonlyAnyRest = readonly [any, ...any];
const readonlyDisplay: number = null! as ReadonlyAnyRest;
type Matches = [string, number] extends [any, ...any] ? 1 : 0;
const matches: Matches = 0;
type MatchesReadonly = [string, number] extends readonly [any, ...any] ? 1 : 0;
const matchesReadonly: MatchesReadonly = 0;
const filled: AnyRest = ["a", 1];
const empty: AnyRest = [];

type Pattern<a> = a extends readonly (infer i)[]
  ? a extends readonly [any, ...any]
    ? { readonly [index in keyof a]: { v: a[index] } }
    : readonly [] | readonly [i, ...i[]]
  : never;
let pattern: Pattern<[string, number]> | string = "x";
pattern = [{ v: "a" }, { v: 1 }];
pattern = [{ v: 1 }, { v: 1 }];
