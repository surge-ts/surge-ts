abstract class A {
    abstract m1(): number;
    abstract m2(): number;
    abstract m3(): number;
    abstract m4(): number;
    abstract m5(): number;
    abstract m6(): number;
}
abstract class One {
    abstract only(): void;
}
abstract class Two {
    abstract first(): void;
    abstract second(): void;
}

class B extends A { }
const C = class extends A {};
const D = class extends One {};
const E = class extends Two {};
function make() {
    return class extends One {};
}
