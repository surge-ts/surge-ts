declare function id<T>(value: T): T;
const pair: [string, number] = id(['a', 1]);
const pairs: [string, number][] = id([['a', 1], ['b', 2]]);
const frozen: readonly [string, number][] = Object.freeze([['a', 1], ['b', 2]]);
declare function wrap<T>(value: T): { value: T };
const wrapped: { value: [string, number] } = wrap(['a', 1]);
const wrong: [string, string] = id(['a', 1]);
