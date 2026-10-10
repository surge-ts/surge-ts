// A truthy `T` is `T & {}`, whose `{}` may be a primitive: it is no right
// operand of `in` (TS2638), as a truthy `unknown` is not.
function check<T>(x: T, u: unknown) {
    if (x && "a" in x) {
    }
    if (u && "a" in u) {
    }
}
function constrained<T extends object | undefined>(x: T) {
    if (x && "a" in x) {
    }
}
