// A callback written with an annotated rest parameter infers each expected
// parameter from the rest's element.
class C { test!: string }
class D extends C { test2!: string }

declare function test<T extends C>(a: (t: T, t1: T) => void): T;
declare function testRest<T extends C>(a: (t: T, t1: T, ...ts: T[]) => void): T;

const r1: D = test((...ts: D[]) => {});
const r2: string = test((...ts: D[]) => {});
const r3: D = testRest((t2, ...t3: D[]) => {});
const r4 = test((first: D, ...rest: D[]) => {});
const r5: number = r4;
