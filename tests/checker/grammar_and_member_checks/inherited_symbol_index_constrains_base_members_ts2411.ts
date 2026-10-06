// @target: es2015
class D1 {
    [Symbol.toStringTag](): { x: string } { return { x: "" }; }
}
class D2 extends D1 {
    [s: symbol]: () => { x: number };
}
class D3 extends D2 {
    m(): void {}
}
