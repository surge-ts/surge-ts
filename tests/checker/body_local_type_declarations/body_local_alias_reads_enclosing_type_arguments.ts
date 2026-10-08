declare function proxyOf<T>(): T;
export function g1<TRecord extends Record<string, unknown>>(rec: TRecord) {
  type H = TRecord extends { todo: number } ? 'yes' : 'no';
  return proxyOf<H>();
}
const q1: 1 = g1({ todo: 1 });
export function g2<TRecord extends Record<string, unknown>>(rec: TRecord) {
  type H = keyof TRecord;
  return proxyOf<H>();
}
const q2: 1 = g2({ todo: 1 });
