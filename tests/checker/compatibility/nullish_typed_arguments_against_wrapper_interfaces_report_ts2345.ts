declare function g(x: String): void;
declare const v: undefined;
g(v);
function foo<T extends String>(x: T): T { return null!; }
function f() {
    var b;
    g(b);
    foo(b);
}
