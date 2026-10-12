function outer<U>(u: U) {
    function bar<V extends U>(x: V): V { return x; }
    return bar;
}
const read = outer("");
read({ length: 3 });
read("text");
