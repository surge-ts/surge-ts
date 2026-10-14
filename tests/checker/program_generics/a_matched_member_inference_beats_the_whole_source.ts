// @strict: true
// `inferToMultipleTypes` infers `P<never>` into `P<R>` first; the whole source
// reaches the naked `R` only at a lower priority, so `R` is `never`.
declare class P<T> {
  private value: T;
}
declare function reject(): P<never>;
declare function andThen<R>(f: () => R | P<R>): P<R>;
export const chained: P<never> = andThen(() => reject());
declare function text(): P<string>;
export const wrapped: P<string> = andThen(() => text());
export const plain: P<number> = andThen(() => 1);
