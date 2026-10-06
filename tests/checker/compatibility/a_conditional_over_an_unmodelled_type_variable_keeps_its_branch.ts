// A contextual signature's type parameter that surge does not model is not a
// value type: an index signature over it still answers a required member.
type SD<S> = S extends { setState: (a: any) => void } ? { devtools: 1 } : never;
interface StoreApi<T> { getState(): T; setState(p: T): void }
type Creator<T> = (api: StoreApi<T>) => T;
type Impl = <T>(fn: Creator<T>) => Creator<T>;
export const impl: Impl = (fn) => (api) => {
  type S = ReturnType<typeof fn> & { [store: string]: ReturnType<typeof fn> };
  (api as StoreApi<S> & SD<S>).devtools;
  return fn(api);
};
