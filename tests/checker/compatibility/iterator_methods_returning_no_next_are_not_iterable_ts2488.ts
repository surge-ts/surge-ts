// @target: es2015
class MyStringIterator {
    [Symbol.iterator]() {
        return this;
    }
}
var v: string;
for (v of new MyStringIterator) { }

class It2 {
    [Symbol.iterator](): It2 { return this; }
}
for (v of new It2) { }
var array = [...new MyStringIterator];

class Good {
    [Symbol.iterator]() { return this; }
    next() { return { done: true as const, value: undefined }; }
}
for (const g of new Good) { }
