type Options<T extends 'array' | 'object'> = T extends 'array' ? { as?: 'object' } : { as: 'array' };

declare class Client {
  execute(query: string, options?: Options<'object'>): 1;
  execute(query: string, options: Options<'array'>): 2;
}

class Base<T> {
  declare value: T;
}

export class Query<T extends { a: unknown }> extends Base<T> {
  private rawQuery = { as: 'object' } as const;
  private query = { as: 'array' } as const;
  private nested = { as: 'object' as const };

  constructor(private client: Client) {
    super();
  }

  run() {
    const { client, rawQuery, query } = this;
    const asObject: 2 = client.execute('', rawQuery);
    const asArray: 1 = client.execute('', query);
    const nested: 2 = this.client.execute('', this.nested);
    return [asObject, asArray, nested];
  }
}

class Holder {
  s = 'x' as const;
  o = { a: 'x' as const };
  r = { as: 'object' } as const;
}

declare const holder: Holder;
export const s: 'y' = holder.s;
export const a: 'y' = holder.o.a;
export const r: 'y' = holder.r.as;
