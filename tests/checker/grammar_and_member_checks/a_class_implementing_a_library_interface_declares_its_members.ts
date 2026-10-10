// @strict: true
// A class implementing a library interface must declare its required members,
// except those every object gets from `Object`.
class C implements Function {
    prototype = null;
    length = 1;
    arguments = null;
    caller = () => { };
}
class D implements Date {}
class H implements PromiseLike<number> {
    then<A = number, B = never>(): PromiseLike<A | B> { return this as any; }
}
class Q implements Iterable<number> {
    [Symbol.iterator]() { return [1][Symbol.iterator](); }
}
class E implements Error { name = ""; message = ""; }
class S implements String {
    toString(): string { return ""; }
}
export {};
