interface CoolArray<E> extends Array<E> {
    hello: number;
}
declare function baz<T>(...args: CoolArray<T>): void;
declare const ca: CoolArray<number>;
baz(1);
baz(1, 2);
baz(...ca);
