type Simplify<T> = { [K in keyof T]: T[K] } & {};

interface Config {
  columns: Record<string, { v: unknown }>;
}

type Infer<TT extends Table> = Simplify<{ [K in keyof TT['_']['columns'] & string]: TT['_']['columns'][K]['v'] }>;

class Table<T extends Config = Config> {
  declare readonly _: { readonly columns: T['columns'] };
  declare readonly $infer: Infer<Table<T>>;
  declare readonly columns: Table<T>['_']['columns'];
}

class Plain {
  declare readonly cols: { z: 1 };
  declare readonly d: Plain['cols'];
}

declare function make<C extends Record<string, { v: unknown }>>(columns: C): Table<{ columns: C }>;

const users = make({ id: { v: 1 as number }, name: { v: '' as string } });
export const inferred: { id: number; name: string } = users.$infer;
export const wrongInferred: string = users.$infer;
export const wrongColumns: string = users.columns;

declare const plain: Plain;
export const wrongCols: string = plain.d;
