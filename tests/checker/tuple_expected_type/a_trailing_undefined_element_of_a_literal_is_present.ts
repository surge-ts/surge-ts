// @strict: true
// A literal's own element whose type includes `undefined` is still present: it
// fills a written tuple's required slot.
declare const n: number | undefined;
export const single: readonly [number | undefined] = [n] as const;
declare const errors: Array<{ message?: string } | undefined>;
export const unique = [...new Map(errors.map((error) => [error?.message, error])).values()];
export const pairs: readonly (readonly [string, { message?: string } | undefined])[] = errors.map(
  (error) => [String(error), error] as const,
);
