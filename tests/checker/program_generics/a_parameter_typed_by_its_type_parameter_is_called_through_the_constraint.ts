// @strict: true
// tsc calls a parameter written as a bare type parameter through that
// parameter's constraint, whatever a call binds it to: `fn(...args)` is the
// constraint's `any`, so the memoized result is `any` too.
function memoize<T extends (...args: any[]) => any>(fn: T) {
  const cache = new Map<string, ReturnType<T>>();
  return (...args: Parameters<T>) => {
    if (cache.has("k")) return cache.get("k");
    const result = fn(...args);
    cache.set("k", result);
    return result;
  };
}
const client = { id: 1 };
declare function use(factory: () => { id: number }): void;
use(memoize(() => client));

function call<T extends () => number>(fn: T) {
  return fn();
}
export const n: string = call(() => 1);
