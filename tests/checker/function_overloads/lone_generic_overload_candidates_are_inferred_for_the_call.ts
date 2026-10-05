// @target: es2015
interface Promise<T> {
    then<U>(success?: (value: T) => U, error?: (error: any) => U, progress?: (progress: any) => void): Promise<U>;
}
declare function testFunction4(x: number, y?: string): Promise<string>;
declare var s4: Promise<string>;
var s4b = s4.then(testFunction4, testFunction4, testFunction4);
declare function t3(x: number): Promise<number>;
declare var s3: Promise<number>;
var s3a = s3.then(t3, t3, t3);
