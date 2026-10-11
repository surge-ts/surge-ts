class NumClass<T extends number> { private value!: T; public numExclusive() { } }
class StrClass<T extends string> { private value!: T; public strExclusive() { } }
function f<E extends { [index: string]: NumClass<number> | StrClass<string> }, K extends keyof E>(e: E, k: K) {
    let entry = e[k];
    const d1: number = entry;
    entry.numExclusive();
    entry.numExclusive;
}
class NP { numExclusive() {} }
class SP { strExclusive() {} }
function g<E extends { [index: string]: NP | SP }, K extends keyof E>(e: E, k: K) {
    let entry = e[k];
    entry.numExclusive;
}
export {};
