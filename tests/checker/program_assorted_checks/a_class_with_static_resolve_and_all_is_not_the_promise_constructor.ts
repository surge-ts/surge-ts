// @strict: true
// Only the lib's `PromiseConstructor` is checked by the `Promise.resolve`,
// `.all`, `.race` and `.reject` rules; a class declaring the same statics is
// called through its own signatures.
export class LoomPromise<T = unknown> {
  constructor(private readonly value?: T) {}
  static resolve<T = void>(value?: T | LoomPromise<T>): LoomPromise<T> {
    return new LoomPromise<T>(value as T);
  }
  static reject(reason?: unknown): LoomPromise<never> {
    return new LoomPromise<never>();
  }
  static all<T>(promises: readonly LoomPromise<T>[]): LoomPromise<T[]> {
    return new LoomPromise<T[]>();
  }
  andThen<R>(onResolved: (value: T) => R | LoomPromise<R>): LoomPromise<R> {
    return new LoomPromise<R>();
  }
}
LoomPromise.resolve(1).andThen((n) => n + 1);
export const rejected: LoomPromise<never> = LoomPromise.reject("no");
const resolved: Promise<number> = LoomPromise.resolve(1);
