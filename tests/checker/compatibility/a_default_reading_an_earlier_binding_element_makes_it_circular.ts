// @noImplicitAny: true
function f1({ value1, test1 = value1.test1 }) {}
function f2({ a = 1, b = a }) {}
function f3({ a, b = a }) {}
function f4({ a, b = () => a }) {}
function f5({ a, b = a } = { a: 1 }) {}
function f6({ a, b = a }: { a: number; b?: number }) {}
function f7([a, b = a]) {}
const f8 = ({ a, b = a }) => {};
function f9({ a, b = String(a) }) {}
export {};
