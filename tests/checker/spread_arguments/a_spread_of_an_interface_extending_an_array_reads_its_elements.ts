// Spreading an interface that extends an array type (directly or through
// other interfaces) contributes the array's elements, as `[...xs]` of the
// array itself does.
interface Values extends ReadonlyArray<number> {}
interface Left extends Values {}
interface Right extends Values {}
interface Combined extends Left, Right {}
interface Derived extends Combined {}
declare const derived: Derived;
const copied: string = [...derived];
export {};
