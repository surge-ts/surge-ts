// @target: es2022
export async function f<T>(x: T, it: AsyncIterable<T | 1>) {
  const y = await x;
  const z: Awaited<T> = y;
  const t: T = y;
  let v: null | Awaited<T> | 1;
  for await (v of it) {}
  type Local = Awaited<T> | 1;
  let w: null | Local;
  for await (w of it) {}
  for await (const u of it) {
    const k: Awaited<T> | 1 = u;
    const bad: T = u;
  }
  return z;
}

export async function g<T extends string, U extends object>(s: T, o: U) {
  const a: T = await s;
  const b: U = await o;
  const c: Awaited<U> = await o;
  return [a, b, c];
}

export function h<T>(x: T) {
  type Local = Awaited<T>;
  const bad: Local = x;
  return bad;
}
