// @surge-compare: messages
namespace Generics {
    export class A<T> { a!: T; }
    export interface I { p: string }
}
declare let x: Generics.A<number>;
const y: { b: string } = x;
const z: Generics.I = 1;
x.nope;
