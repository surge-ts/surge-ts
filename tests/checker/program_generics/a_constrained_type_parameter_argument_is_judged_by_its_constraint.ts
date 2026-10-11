// A declaration's own type parameter written as a type argument is judged by
// its constraint (`getBaseConstraintOrType`): one that does not satisfy the
// referenced parameter's constraint is TS2344, as an unconstrained one is.
type Text<T extends string> = { text: T };
type FromNumber<U extends number> = Text<U>;
interface Holder<U extends number> {
    held: Text<U>;
}
function take<U extends number>(value: Text<U>) {
    return value;
}
type Unconstrained<U> = Text<U>;
type Literal<U extends "a" | "b"> = Text<U>;
type Wide<U extends string | number> = Text<U>;
class Box<U extends boolean> {
    method(value: Text<U>) {
        return value;
    }
}
export type { FromNumber, Holder, Unconstrained, Literal, Wide };
export { take, Box };
