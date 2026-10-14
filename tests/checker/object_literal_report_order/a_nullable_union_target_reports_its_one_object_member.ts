// @strict: true
// `isRelatedTo` relates an object to a union of at most three members by the
// member left once `null` and `undefined` are taken out.
type Left = { left: number; shared: string };
type Right = { right: string; shared: string };
declare function f(x: Left | undefined): void;
f({ shared: "s" });
export const a: Left | undefined | null = { shared: "s" };
export const b: Left | void = { shared: "s" };
export const c: Left | Right | undefined = { shared: "s" };
export function g(): Left | undefined {
  return { shared: "s" };
}
export const d: { inner: Left | undefined } = { inner: { shared: "s" } };
