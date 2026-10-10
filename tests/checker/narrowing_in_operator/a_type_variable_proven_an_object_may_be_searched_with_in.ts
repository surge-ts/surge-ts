// `typeof x === "object"` narrows a truthy `T & {}` to `T & object`, which
// `in` accepts; a bare truthy `T` may still be a primitive (TS2638).
function f<T>(x: T) {
    if (x && "a" in x) {
        x;
    }
    if (x && typeof x === "object" && "a" in x) {
        x.a;
    }
    if (x && typeof x === "object" && "a" in x && "b" in x) {
        x.b;
    }
}
