type U = { a: number } | { b: string };
type N = number;
type O = { a: number } & { b: string };

interface I1 extends U { x: string }
interface I2<T> extends T { x: string }
interface I3 extends N { x: string }
interface I4 extends O { x: string }
