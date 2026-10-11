declare class A<T, R = undefined> { next(): T; other(): R; }
type ACtor = typeof A;
interface X extends ACtor { from(): void }
declare const x: X;
new x<number>();
x.from();
