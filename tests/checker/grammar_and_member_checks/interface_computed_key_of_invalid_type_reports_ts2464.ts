namespace Foo {
    export enum Enum { A = "a", B = "b" }
}
const key = "k";
declare const sym: unique symbol;
declare const obj: { n: number };
interface I {
    [Symbol.keyFor]: string;
    [Symbol.iterator](): Iterator<number>;
    [Foo.Enum]: 0;
    [key]: string;
    [sym]: number;
    [obj.n]: boolean;
}
