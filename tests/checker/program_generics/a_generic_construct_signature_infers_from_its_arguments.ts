// @strict: false
// `new` on a value whose construct signature is generic infers the type
// arguments from the arguments, as a call does: with no common supertype the
// first candidate wins and the later arguments are checked against it.
interface Triple {
    new <T>(a: T, b: T, c: T): T;
}
declare var triple: Triple;
var first = new triple('', 0, []);

declare const pair: new <T>(a: T, b: T) => T;
var second = new pair(1, 2);
var third = new pair('', 0);
