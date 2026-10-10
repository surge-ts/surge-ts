// Under `T extends { [K in keyof T]: string }` the constraint of `keyof T` is
// `keyof T` again, which relates only as maybe: no literal is assignable to it.
function f<T extends { [K in keyof T]: string }>(k: keyof T) {
    k = 42;
    k = "hello";
}
