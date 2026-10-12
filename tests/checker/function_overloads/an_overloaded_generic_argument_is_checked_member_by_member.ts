interface IPromise<T> {
    then<U>(success?: (value: T) => U): IPromise<U>;
}
declare function overloaded<T>(x: T): IPromise<T>;
declare function overloaded<T>(x: T, y: T): Promise<T>;
declare function h(cb: (value: number) => Promise<unknown>): void;
h(overloaded);
declare function g<U>(cb: (value: number) => U): U;
const inferred: boolean = g(overloaded);
