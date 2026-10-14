// @strict: true
// A signature is bound before the module's values; a written `typeof all`
// over a call-inferred value reads the value's type, here the error type, so
// callbacks over it are untyped.
declare const client: { $on(): void };
interface QueryProcedure<TDef> {
  _def: TDef;
}
declare function make<T>(f: () => T | Promise<T>): QueryProcedure<{ output: T }>;
export const all = make(() => {
  return client.task;
});
type Output<T> = T extends QueryProcedure<infer D extends { output: unknown }> ? D["output"] : never;
declare function getData(): Output<typeof all> | undefined;
const data = getData();
if (data) data.map((row) => row.id);
const list = data ?? [];
list.map((row) => row.id);
