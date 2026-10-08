interface Retryer<T> { promise: Promise<T>; start(): Promise<T> }
interface Cfg<TData = unknown> { fn: () => TData | Promise<TData>; initialPromise?: Promise<TData> }
declare function createRetryer<TData = unknown>(config: Cfg<TData>): Retryer<TData>;
export class Q<TData> {
  #retryer?: Retryer<TData>;
  a(ip: unknown, fn: unknown) {
    const retryer = (this.#retryer = createRetryer({ initialPromise: ip as Promise<TData> | undefined, fn: fn as () => Promise<TData> }));
    this.#retryer = createRetryer({ initialPromise: ip as Promise<TData> | undefined, fn: fn as () => Promise<TData> });
    const r2: Retryer<TData> = createRetryer({ initialPromise: ip as Promise<TData> | undefined, fn: fn as () => Promise<TData> });
    const r3 = createRetryer({ initialPromise: ip as Promise<TData> | undefined, fn: fn as () => Promise<TData> });
    const p: 1 = r3;
  }
}
