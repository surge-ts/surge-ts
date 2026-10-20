// tsc's `getInstantiationExpressionType`: `make<string>` is its one
// signature instantiated, which later calls are checked against.
declare function make<T>(value: T): T[];
declare function pair<T, U = number>(a: T, b: U): [T, U];
declare const factories: { make<T>(value: T): T[] };
const makeStrings = make<string>;
export const bad: number[] = makeStrings('a');
export const bad2 = makeStrings(1);
const m2 = factories.make<string>;
export const bad3 = m2(2);
const p1 = pair<string>;
export const p = p1('a', 2);
export const bad4 = p1('a', 'b');
