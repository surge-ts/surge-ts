class Base { foo!: string; }
class Derived extends Base { bar!: string; }
interface Box<T> { v: T }

declare var a1: (y: number) => () => string;
declare var b1: <T>(y: string) => () => T;
a1 = b1;

declare var a2: (y: number) => (r: Base) => Derived;
declare var b2: <T extends Base, U extends Derived>(y: string) => (r: T) => U;
a2 = b2;

declare var f1: <T extends string>() => T;
let x1: string = f1;

declare var bx: <T extends Base>() => Box<T>;
let bb1: Box<Base> = bx;
let bb2: Box<string> = bx;

declare var src: <T extends Base>() => (x: T) => T;
declare var tgt: <T extends string>(x: T) => T;
tgt = src;

function body<T extends string>(g: () => T) {
    let s: string = g;
    s = g;
}
