interface StringIndexed {
    [Symbol.iterator]: number;
    [s: string]: string;
}

const sym = Symbol();

interface AA {
    a?: string;
    b?: number;
    [key: symbol]: string;
}

const aa: AA = { [sym]: "123" };
