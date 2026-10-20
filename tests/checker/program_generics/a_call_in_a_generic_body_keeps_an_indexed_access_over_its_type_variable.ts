declare function get<T, K extends keyof T>(obj: T, key: K): T[K];

export function read<U extends { a: number }>(u: U) {
  const value = get(u, "a");
  const count: number = value;
  const label: string = value;
  return [count, label];
}

declare function pick<T>(obj: T): T extends { b: infer B } ? B : never;

export function readB<V extends { b: string }>(v: V) {
  const b = pick(v);
  const fine: string = b;
  return fine;
}
