// @strict: true
// @surge-compare: messages
// @surge-compare: spans
function bar<T extends unknown[]>(callback: (this: 1, ...args: T) => void) {
    callback.bind(2);
}
function baz<T extends 1 | 2>(callback: (this: 1, ...args: T extends 1 ? [unknown] : [unknown, unknown]) => void) {
    callback.bind(2);
}
function fixed<A extends string>(callback: (this: 1, a: A) => void) {
    callback.bind(2);
}
function anyRest<T extends any[]>(callback: (this: 1, ...args: T) => void) {
    callback.bind(2);
}
function labeled<T extends [a: string] | [b: number, c: string]>(callback: (this: 1, ...args: T) => void) {
    (callback).bind(2);
}
function readonlyTuple<T extends readonly [string, number]>(callback: (this: 1, ...args: T) => void) {
    callback.bind(2);
}
declare const isString: (this: 1, x: unknown) => x is string;
const isNumber: (this: 1, x: unknown) => x is number = isString;
declare const pair: (this: 1, a: string, b: number) => void;
const strings: (this: 1, a: string, b: string) => void = pair;
declare const pattern: (this: 1, { a }: { a: string }) => void;
const numbers: (this: 1, { a }: { a: number }) => void = pattern;
