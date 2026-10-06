namespace M {
    namespace Symbol { }
    const x = Symbol.iterator;
    class C { [Symbol.iterator]() { } }
    namespace N { export type T = 1; }
    const y = N;
}
