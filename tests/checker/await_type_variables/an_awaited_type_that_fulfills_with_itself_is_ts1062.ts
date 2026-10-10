// @strict: true
// @target: es2017
// Awaiting a type whose promise fulfills with the type being awaited never
// reaches a settled value (TS1062 at the `await`): a union reached again
// through one of its own promises, directly or after narrowing.
type T1 = 1 | Promise<T1> | T1[];

export async function myFunction(param: T1) {
  const awaited = await param
}

type EffectResult =
  | (() => EffectResult)
  | Promise<EffectResult>;

export async function handleEffectResult(result: EffectResult) {
  if (result instanceof Function) {
    await handleEffectResult(result());
  } else if (result instanceof Promise) {
    await handleEffectResult(await result);
  }
}

type Plain = 1 | Promise<2> | Promise<Promise<3>>;
export async function fine(value: Plain) {
  const settled = await value;
}
