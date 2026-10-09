// @strict: true
export const keys: 0 = null! as keyof never;
export const mapped: 0 = null! as { [K in keyof never]?: true };
type Homomorphic<T> = { [K in keyof T]?: true | undefined } | undefined;
export const homomorphic: 0 = null! as Homomorphic<never>;
type ThroughAccess<T extends { relations: unknown }> = { [K in keyof T['relations']]?: true };
export const throughAccess: 0 = null! as ThroughAccess<{ relations: never }>;
type Empty = {};
type Config<T extends { relations: unknown }> = {
  with?: { [K in keyof T['relations']]?: true } | undefined;
};
type With = Config<{ relations: Empty[keyof Empty] }>['with'];
export const withBranch: With extends Record<string, unknown> ? 1 : 2 = 1;
