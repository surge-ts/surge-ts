// @strict: true
declare const matcher: {
  with<const p extends string>(pattern: p, handler: (value: p) => 1): 'one';
  with<const p1 extends readonly unknown[], const p2 extends readonly unknown[]>(
    first: p1,
    second: p2,
    handler: (value: p1 | p2) => 2,
  ): 'two';
  with(pattern: unknown, predicate: (value: unknown) => boolean, handler: () => 3): 'three';
};
export const two: 'two' = matcher.with([1], [1, 2], () => 2);
export const wrong: 'three' = matcher.with([1], [1, 2], () => 2);
