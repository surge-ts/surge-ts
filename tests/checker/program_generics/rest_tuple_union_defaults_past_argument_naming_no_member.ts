// @strict: true
declare class Pool { p: 1 }
declare class Client { c: 1 }
type NodePgClient = Pool | Client;
interface DrizzleConfig<T> { schema?: T; logger?: boolean }
declare function drizzle<
  TSchema extends Record<string, unknown> = Record<string, never>,
  TClient extends NodePgClient = Pool,
>(
  ...params:
    | [TClient | string]
    | [TClient | string, DrizzleConfig<TSchema>]
    | [DrizzleConfig<TSchema> & ({ client: TClient } | { connection: string })]
): { schema: TSchema } & { $client: NodePgClient extends TClient ? Pool : TClient };
export const fromInstance: 0 = drizzle(new Client());
const client = new Client();
export const fromBinding: 0 = drizzle(client);
export const fromString: 0 = drizzle('url');
