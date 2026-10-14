// @strict: true
// The contextual return type gives `then`'s `TResult1 | TResult2` the whole
// `string | undefined` at a low priority; the callback's own return decides.
type Resolver = (image: string) => string | undefined | PromiseLike<string | undefined>;
declare function setResolver(resolver: Resolver | undefined): void;
declare function manifest(base: string): Promise<Record<string, string> | undefined>;
setResolver((image) => {
  if (image === "") return undefined;
  return manifest("/").then((baked) => {
    const file = baked?.[image];
    return file === undefined ? undefined : `/${file}`;
  });
});
export const typed: Promise<number> = manifest("/").then((baked) => baked?.["x"]);
