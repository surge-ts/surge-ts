// @strict: true
// `bind(thisArg)` binds no argument, so a callback whose rest is typed by a
// type variable of the body still has its written `this` checked (TS2769).
function bar<T extends unknown[]>(callback: (this: 1, ...args: T) => void) {
    callback.bind(1);
    callback.bind(2);
}
function baz<T extends 1 | 2>(callback: (this: 1, ...args: T extends 1 ? [unknown] : [unknown, unknown]) => void) {
    callback.bind(1);
    callback.bind(2);
}
