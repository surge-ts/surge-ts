// @strict: true
declare function pair<A, B>(a: A, b: B): void;
type Partial1 = typeof pair<string>;
type Returned = ReturnType<Partial1>;
type Taken = Parameters<Partial1>;
type Whole = typeof pair<string, number>;
type WholeReturned = ReturnType<Whole>;
