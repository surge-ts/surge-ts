// @strict: true
interface Config<S> { schema?: S; logger?: boolean }
type ModeConfig<S> = Omit<Config<S>, 'schema'> & ({ schema: S; mode: 'a' } | { schema?: undefined; mode?: 'a' });
declare class Pool { p: 1 }
declare function connect<S extends Record<string, unknown> = Record<string, never>>(
  ...params: [Pool | string] | [Pool | string, ModeConfig<S>] | [ModeConfig<S> & { client: Pool }]
): { schema: S };
export const fromPool: string = connect(new Pool()).schema;
