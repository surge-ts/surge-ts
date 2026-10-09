// @strict: true
// @lib: es2023
const listOf = <T,>(x: T) => [x];
const listed: string[] = listOf(1);
const wrapper = function <T>(x: T) { return { value: x }; };
const wrapped: { value: string } = wrapper(1);
const helpers = {
  make<T>(x: T) { return [x]; },
  pair: <A, B>(a: A, b: B) => [a, b] as const,
};
const made: string[] = helpers.make(1);
const paired: number = helpers.pair(1, "a");
const kept: number[] = listOf(1);
