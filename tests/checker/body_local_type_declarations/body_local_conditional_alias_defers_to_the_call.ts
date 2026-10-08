type IntersectionError<TKey extends string> = `The property '${TKey}' collides`;
type ProtectedIntersection<TType, TWith> = keyof TType & keyof TWith extends never ? TType & TWith : IntersectionError<string & keyof TType & keyof TWith>;
type Deco<TRecord> = { [TKey in keyof TRecord]: { fetch(): TRecord[TKey] } };
declare function proxyOf<T>(): T;
export function helpers<TRecord extends Record<string, unknown>>(rec: TRecord) {
  type H = ProtectedIntersection<{ queryClient: 1; dehydrate: 2 }, Deco<TRecord>>;
  return proxyOf<H>();
}
const h = helpers({ todo: 1 });
const p: 1 = h;
