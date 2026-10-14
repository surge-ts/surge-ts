// @strict: true
// @skipLibCheck: true
// A script that adds its own `then` to the global `Promise` (roblox-ts's
// compiler types) merges into the lib's: awaiting still unwraps through
// either signature, and an async return is related to the awaited type.
// @filename: globals.d.ts
interface PromiseLike<T> {
	then<TResult1 = T, TResult2 = never>(
		this: Promise<T>,
		onResolved?: ((value: T) => TResult1 | Promise<TResult1>) | void,
		onRejected?: ((reason: any) => TResult2 | Promise<TResult2>) | void,
	): Promise<TResult1 | TResult2>;
}
interface Promise<T> {
	then<TResult1 = T, TResult2 = never>(
		this: Promise<T>,
		onResolved?: ((value: T) => TResult1 | Promise<TResult1>) | void,
		onRejected?: ((reason: any) => TResult2 | Promise<TResult2>) | void,
	): Promise<TResult1 | TResult2>;
	andThen(): void;
}
// @filename: a.ts
declare function list(): Promise<string[]>;
export async function count(): Promise<number> {
  const xs = await list();
  return xs.length;
}
export async function first(): Promise<string | undefined> {
  const xs = await list().catch(() => undefined);
  if (xs && xs.some((name) => name.length > 1)) return xs[0];
  return undefined;
}
