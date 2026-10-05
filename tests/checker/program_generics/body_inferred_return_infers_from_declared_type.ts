function createDeferred<TValue = void>() {
  let resolve: (value: TValue) => void;
  const promise = new Promise<TValue>((res) => { resolve = res; });
  return { promise, resolve: resolve! };
}
type Deferred<TValue> = ReturnType<typeof createDeferred<TValue>>;
export async function f<THead>() {
  let d: null | Deferred<THead> = createDeferred();
  const head = await d.promise;
  let s: Deferred<string> | null = createDeferred();
  s.resolve("ok");
  const n: number = await s.promise;
  return head;
}
