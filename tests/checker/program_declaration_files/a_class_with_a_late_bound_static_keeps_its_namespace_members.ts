// @strict: true
// @target: es2022
const entityKind = Symbol.for("entityKind");
type EntityClass<T> = (abstract new (...args: any[]) => T) & { [entityKind]: string };
declare function is<T extends EntityClass<any>>(value: any, type: T): value is InstanceType<T>;
export class Sql<T = unknown> {
  static readonly [entityKind]: string = "Sql";
  decoder = 1;
  declare _: { type: T };
}
export namespace Sql {
  export class Aliased<T = unknown> {
    static readonly [entityKind]: string = "Sql.Aliased";
    fieldAlias = "alias";
    declare _: { type: T };
  }
}
export function decoderOf(entry: Sql | Sql.Aliased): unknown {
  if (is(entry, Sql.Aliased)) {
    return entry.fieldAlias;
  }
  return entry.decoder;
}
