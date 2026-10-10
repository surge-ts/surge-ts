// @strict: true
// A type variable inferred for a type parameter relates through its own
// constraint; one that cannot satisfy the parameter's constraint gives way
// to that constraint (`getInferredType`), which the arguments then fail.
function foo2<T extends Date>(a: (x: T) => T, b: (x: T) => T) { }
function other3<T extends RegExp>(x: T) {
    foo2((a: T) => a, (b: T) => b);
}
function other4<T extends Date>(x: T) {
    foo2((a: T) => a, (b: T) => b);
}
