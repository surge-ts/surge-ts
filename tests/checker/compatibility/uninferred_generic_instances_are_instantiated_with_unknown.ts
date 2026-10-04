interface I { id: number }
class D<T> { source!: T }
var anInterface: I = new D();
declare const b: D<unknown>;
const n: number = b;
const fine: { source: unknown } = b;
