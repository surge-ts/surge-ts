// @strict: true
// @lib: es2023
function read<T>(xs: T[], i: number) {
  const literal: number = xs[0];
  const computed: number = xs[i];
  const inferred = xs[0];
  const fromInferred: number = inferred;
  return xs[0];
}
const returned: number = read([true], 0);
function readonlyRead<T>(xs: readonly T[]) {
  const literal: number = xs[0];
}
function optionalRead<T>(xs?: T[]) {
  const element: number = xs?.[0];
}
function kept<T>(xs: T[], fallback: T): T {
  return xs[0] ?? fallback;
}
