// @strict: true
// The callback's returned call is checked against the contextual return type
// the generic call infers from its own context, so the inner call's type
// argument comes from there rather than from its default.
interface Statement {
  raw<T = unknown[]>(options: { columnNames: true }): Promise<[string[], ...T[]]>;
  raw<T = unknown[]>(options?: { columnNames?: false }): Promise<T[]>;
}
declare function cache<T>(query: () => Promise<T>): Promise<T>;
declare const statement: Statement;
declare function rows<T = unknown[]>(): Promise<T[]>;
export async function values<T extends any[] = unknown[]>(): Promise<T[]> {
  return await cache(async () => {
    return statement.raw();
  });
}
export function expressionBody<T>(): Promise<T[]> {
  return cache(() => rows());
}
export const annotated: Promise<string[][]> = cache(() => rows());
export const unrelated: Promise<string> = cache(() => rows());
