declare namespace NX { export const a = "hello"; export const b = 1; }
const t1: typeof NX.a = "x";
const t2: typeof NX.b = 2;
type TNX = typeof NX;
interface I12 extends TNX { a: number }
