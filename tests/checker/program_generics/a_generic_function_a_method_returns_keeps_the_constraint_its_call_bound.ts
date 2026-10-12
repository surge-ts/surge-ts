class Plain {
    read<U>(u: U) {
        function bar<V extends U>(y: V) { return y; }
        return bar;
    }
}
const plain = new Plain().read("");
plain({ length: 3 });
plain("text");

class Sized<T extends { length: number }> {
    constructor(x: T) { }
    read<U extends T>(u: U) {
        function bar<V extends U>(x: V) { return x; }
        return bar;
    }
}
const sized = new Sized({ length: 2 }).read("");
sized({ length: 3, charAt: (x: number) => { "" } });
sized("text");
