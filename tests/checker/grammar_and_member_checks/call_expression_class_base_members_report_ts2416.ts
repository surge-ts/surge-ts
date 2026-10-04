type Constructor<T> = new () => T;
declare function Constructor<T>(): Constructor<T>;

type T1 = { a: number };
type T2 = { b: number; c: string };

class C1 extends Constructor<T1>() { a: string = "" }
class C2 extends Constructor<T2>() { b: number = 0; c: number = 0 }
class C3 extends Constructor<T1>() { a: number = 1 }
