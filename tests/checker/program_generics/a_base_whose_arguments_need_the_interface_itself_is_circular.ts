// tsc resolves a heritage reference's type arguments eagerly; one that indexes
// the interface, takes its keys or maps over them needs the members its bases
// are still computing (TS2310), and the non-generic alias on the way (TS2456)
// or the mapped type ranging over those keys (TS2313) is circular as well.
type M<T> = { value: T };
interface M2 extends M<M3> {}
type M3 = M2[keyof M2];

type X<T> = { [K in keyof T]: string } & { b: string };
interface Y extends X<Y> {
    a: "";
}

type Box<T> = { inner: T };
interface Fine extends Box<Fine> {
    a: string;
}
