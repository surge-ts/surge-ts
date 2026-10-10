// @target: es2015
// A base class's `symbol` index constrains the members a derived class
// declares, as an interface base's does.
class Base {
    [s: symbol]: () => { x: number };
}
class Derived extends Base {
    [Symbol.toStringTag]() {
        return { x: "" };
    }
}
class Fits extends Base {
    [Symbol.iterator]() {
        return { x: 1 };
    }
}
