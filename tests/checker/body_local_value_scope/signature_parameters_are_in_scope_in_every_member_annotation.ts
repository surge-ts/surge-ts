type Named = { name: string };
class C {
    m({ name: alias }: Named & { o: typeof alias }) {}
}
declare class D {
    m({ name: alias }: Named, p: typeof alias): void;
}
interface I {
    m(a: typeof b, b: string): void;
}
type T = (a: { x: typeof a }) => void;
type U = (a: number, b: typeof a) => typeof b;
const u: U = (a, b) => b;
const bad: string = u(1, 2);
