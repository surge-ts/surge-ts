// @surge-compare: messages
declare let a: { children?: boolean };
export function annotated<CP extends { o: object }>(props: CP & { children?: boolean }) {
    props = a;
}

interface F<P> {
    (props: P & { children?: boolean }): void;
    propTypes: { [K in keyof P]: null extends P[K] ? K : K };
}
declare function g(f: F<unknown>): void;
export function passed<CP extends { o: object }>(f: F<CP>) {
    g(f);
}

export function nested<T>(list: readonly (T & { x: number })[], maybe: (T & { x: number }) | undefined) {
    let n: number = list;
    let m: number = maybe;
}
