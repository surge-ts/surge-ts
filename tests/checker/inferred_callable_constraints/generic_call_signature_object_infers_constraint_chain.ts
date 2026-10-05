class A { foo(): string { return ""; } }
declare var d: { <U extends T, T extends A>(): U };
d().notHere;
const r = d();
r.alsoNotHere;
declare var h: { <T>(x: T): T; p: number };
const y1: string = h(1);
type H = { <T>(x: T): T };
declare var h2: H;
const y2: string = h2(1);
