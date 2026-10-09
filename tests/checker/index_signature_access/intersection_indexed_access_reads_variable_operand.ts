// @strict: true
interface Config { execute: unknown; iterator: unknown }
abstract class PreparedQuery<T extends Config> {
  abstract execute(placeholderValues?: Record<string, unknown>): Promise<T['execute']>;
}
interface HKT { readonly config: unknown; readonly type: unknown }
interface Plain extends HKT { type: PreparedQuery<Config> }
type Assume<T, U> = T extends U ? T : U;
type Kind<TKind extends HKT, TConfig extends Config> =
  Assume<(TKind & { readonly config: TConfig })['type'], PreparedQuery<TConfig>>;
type Prepared<TKind extends HKT> = (TKind & { readonly config: 1 })['type'];

export class Query<P extends Plain> {
  prepared!: (P & { readonly config: 1 })['type'];
  aliased!: Prepared<P>;
  run: (P & { readonly config: 1 })['type']['execute'] = (placeholderValues) =>
    this.prepared.execute(placeholderValues);
  kind: Kind<P, { execute: number; iterator: never }>['execute'] = (values) => null!;
  check() {
    const prepared: 0 = this.prepared;
    const execute: 0 = this.prepared.execute;
    this.aliased.missing;
  }
}
