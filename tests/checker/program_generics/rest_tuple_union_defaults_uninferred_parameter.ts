// @strict: true
declare class Pool { p: 1 }
declare class Client { c: 1 }
type NodePgClient = Pool | Client;
interface DrizzleConfig<T> { schema?: T }
declare function drizzle<TSchema extends Record<string, unknown> = Record<string, never>, TClient extends NodePgClient = Pool>(
  ...params: [TClient | string] | [TClient | string, DrizzleConfig<TSchema>]
): { schema: TSchema } & { $client: NodePgClient extends TClient ? Pool : TClient };
const db = drizzle(new Client());
export const schema: { wrong: true } = db.schema;
export const client: Pool = db.$client;
