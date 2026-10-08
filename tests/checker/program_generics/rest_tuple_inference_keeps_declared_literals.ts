type Act = 'cascade' | 'restrict';
interface Cfg { ref: () => number; actions: { onUpdate?: Act } }
declare function iife<T extends unknown[], U>(fn: (...args: T) => U, ...args: T): U;
declare function iife2<T extends unknown[]>(...args: T): T;
export function f(cfg: Cfg, a: { onUpdate?: Act }, b: Act) {
  const r1: 1 = iife2(a);
  const r2: 1 = iife2(b);
  const r3: 1 = iife((x) => x, a);
}
