// @strict: true
type Simplify<T> = { [K in keyof T]: T[K] } & {};
type Model = Simplify<{ id: number } & { name?: string }>;
export const missingOne: Model = {};
export const complete: Model = { id: 1 };
type Pair = { a: 1 } & { b: 2 };
export const realIntersection: Pair = {};
