function fun(x = new FooIterator()) { return x; }
function fun2([a] = new FooIterator()) { return a; }
interface I {}
function g(x = I) { return x; }
class FooIterator { [Symbol.iterator]() { return [1][Symbol.iterator](); } }
