// @strict: true
// @lib: es2023
declare function stars<V extends string>(arg: `*${V}*`): V;
let unmatched = stars("hello");
let matched = stars("*hello*");
const widened: number = matched;
const literal = stars("*a*");
const narrow: "b" = literal;
declare function split<A extends string, B extends string>(arg: `${A}${B}`): [A, B];
const parts: number = split("xyz");
declare function route<P extends string>(path: `/users/${P}`): P;
const id: number = route("/users/42");
declare function noInfer<V extends string>(arg: `*${V}*`): void;
noInfer("plain");
