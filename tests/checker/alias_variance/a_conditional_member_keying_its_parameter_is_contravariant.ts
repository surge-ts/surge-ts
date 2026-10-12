interface Covariant<T> {
    foo: T extends string ? T : number;
}
interface Contravariant<T> {
    foo: T extends string ? keyof T : number;
}
interface Invariant<T> {
    foo: T extends string ? keyof T : T;
}
interface Keys<T> {
    keys: keyof T;
}
function covariant<A, B extends A>(a: Covariant<A>, b: Covariant<B>) {
    a = b;
    b = a;
}
function contravariant<A, B extends A>(a: Contravariant<A>, b: Contravariant<B>) {
    a = b;
    b = a;
}
function invariant<A, B extends A>(a: Invariant<A>, b: Invariant<B>) {
    a = b;
    b = a;
}
function keys<A, B extends A>(a: Keys<A>, b: Keys<B>) {
    a = b;
    b = a;
}
