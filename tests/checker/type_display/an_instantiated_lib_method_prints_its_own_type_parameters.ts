// @surge-compare: messages
// @strict: false
declare const p: Promise<number>;
declare const pl: PromiseLike<number>;
interface IPromise<T> { then<U>(success?: (value: T) => U): IPromise<U>; }
export const then: string = p.then;
export const caught: string = p.catch;
export const like: string = pl.then;
export const related: IPromise<string> = p;
