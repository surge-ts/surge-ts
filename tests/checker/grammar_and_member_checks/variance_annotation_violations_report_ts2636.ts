type Covariant<in T> = {
    x: T;
}

type Contravariant<out T> = {
    f: (x: T) => void;
}

type Invariant<in T> = {
    f: (x: T) => T;
}

type Fine<out T> = {
    x: T;
}

interface Box<in T> {
    value: T;
}
