interface IPromise<T> {
    then<U>(success?: (value: T) => IPromise<U>, error?: (error: any) => IPromise<U>, progress?: (progress: any) => void): IPromise<U>;
    then<U>(success?: (value: T) => IPromise<U>, error?: (error: any) => U, progress?: (progress: any) => void): IPromise<U>;
    then<U>(success?: (value: T) => U, error?: (error: any) => IPromise<U>, progress?: (progress: any) => void): IPromise<U>;
    then<U>(success?: (value: T) => U, error?: (error: any) => U, progress?: (progress: any) => void): IPromise<U>;
}
declare function testFunction(): IPromise<number>;
declare function sIPromise(x: any): IPromise<string>;
declare function nIPromise(x: any): IPromise<number>;
declare var unknownPromise: IPromise<unknown>;
unknownPromise.then(testFunction, sIPromise);
unknownPromise.then(testFunction, sIPromise, nIPromise);
declare function testFunction10<T>(cb: <U>(a: U) => U): IPromise<T>;
testFunction10(x => x).then(testFunction, sIPromise, nIPromise);
declare function written(success?: (value: unknown) => string): void;
written(testFunction);
declare function inferred<U>(error?: (error: any) => IPromise<U>, success?: (value: unknown) => U): IPromise<U>;
inferred(sIPromise, testFunction);
