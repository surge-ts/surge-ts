declare const callback: (() => void) | null;
declare const ctor: Function;
if (callback instanceof ctor) {
    callback();
} else {
    callback();
}
declare const value: string | { a: 1 } | null;
if (value instanceof ctor) {
    const object: { a: 1 } = value;
}
