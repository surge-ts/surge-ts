type Prepared<T> = { execute: (placeholderValues?: Record<string, unknown>) => Promise<T> };
type Kind<HKT, T> = (HKT & { readonly config: T })['type' & keyof HKT];

abstract class QueryPromise<T> {
  abstract execute(): Promise<T>;
}

interface PreparedHKT {
  type: Prepared<number>;
}

export class Delete<TDynamic extends boolean> extends QueryPromise<number> {
  declare readonly _: { readonly hkt: PreparedHKT; readonly dynamic: TDynamic };

  prepare(): Kind<this['_']['hkt'], { x: 1 }> {
    return null!;
  }

  override execute: ReturnType<this['prepare']>['execute'] = (placeholderValues) => {
    return this.prepare().execute(placeholderValues);
  };

  wrongPrepare: this['prepare'] = 5 as number;
}

type Instance = Delete<true>;
export const prepared: number = null! as ReturnType<Instance['prepare']>;
export const executor: number = null! as ReturnType<Instance['prepare']>['execute'];
