// A written `unknown` type argument is a type: `Box<unknown>` is no
// `Box<string>`.
interface Box<T> {
    x: T;
}
declare let unknownBox: Box<unknown>;
declare let stringBox: Box<string>;
stringBox = unknownBox;
unknownBox = stringBox;
type Pair<A, B> = { first: A; second: B };
declare let loose: Pair<unknown, number>;
declare let tight: Pair<string, number>;
tight = loose;
loose = tight;
