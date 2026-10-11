// @noImplicitAny: true
// @target: es2015
class ReturnsHead {
    [Symbol.iterator]() {
        return v1;
    }
}
for (var v1 of new ReturnsHead) { }

class NextReturnsHead {
    next() {
        return { done: true, value: v2 };
    }
    [Symbol.iterator]() {
        return this;
    }
}
for (var v2 of new NextReturnsHead) { }

class NextReturnsOther {
    next() {
        return { done: true, value: 1 };
    }
    [Symbol.iterator]() {
        return this;
    }
}
for (var v3 of new NextReturnsOther) { }
