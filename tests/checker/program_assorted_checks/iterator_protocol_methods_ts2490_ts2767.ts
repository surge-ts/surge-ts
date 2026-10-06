class StringNext {
    next() {
        return "";
    }
    [Symbol.iterator]() {
        return this;
    }
}

var v: string;
for (v of new StringNext) { }

class NumericReturn {
    next() {
        return { done: false, value: "" };
    }
    return = 0;
    [Symbol.iterator]() {
        return this;
    }
}

for (var w of new NumericReturn) { }

class Fine {
    next() {
        return { done: false, value: "" };
    }
    [Symbol.iterator]() {
        return this;
    }
}

for (const x of new Fine) { }
const spread = [...new StringNext];
