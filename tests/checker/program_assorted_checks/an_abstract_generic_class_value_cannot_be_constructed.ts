// @strict: true
// @target: esnext
abstract class A<T> {
    abstract next(): T;
}
declare const viaTypeof: typeof A;
new viaTypeof<number>();
const alias = A;
new alias();
new Iterator<number>();
