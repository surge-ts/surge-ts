class A { propA: number = 0; }
class C extends A { propC: number = 0; }
declare function isC(p1: unknown): p1 is C;
declare function funD<T>(p1: (p1: unknown) => p1 is T, p2: unknown): p2 is T;
declare let a: A;
if (funD(isC, a)) {
    a.propC;
    a.missing;
}
