interface Base<T, U> {
    x: T;
    y: U;
}

interface BaseConstructor {
    new (x: string, y: string): Base<string, string>;
    new <T>(x: T): Base<T, T>;
    new <T>(x: T, y: T): Base<T, T>;
    new <T, U>(x: T, y: U): Base<T, U>;
}

declare function getBase(): BaseConstructor;

class D1 extends getBase() {}
class D2 extends getBase() <number> {}
class D3 extends getBase() <string, number> {}
class D4 extends getBase() <string, string, string> {}

interface BadBaseConstructor {
    new (x: string): Base<string, string>;
    new (x: number): Base<number, number>;
}

declare function getBadBase(): BadBaseConstructor;

class D5 extends getBadBase() {}
