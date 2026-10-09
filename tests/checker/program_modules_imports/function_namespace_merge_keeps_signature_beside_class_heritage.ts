// @strict: true
// @filename: driver.ts
export interface DrizzleConfig<T> { schema?: T }
export declare class Client { c: 1 }
class Base<T> { base!: T }
export class Database<TSchema> extends Base<TSchema> { db!: TSchema }
export function drizzle<TSchema = Record<string, never>>(
  ...params: [Client | string] | [Client | string, DrizzleConfig<TSchema>]
): Database<TSchema> & { $client: Client } {
  return null!;
}
export namespace drizzle {
  export function mock(): { mocked: true } { return null!; }
}
// @filename: a.ts
import { drizzle, Client } from './driver';
export const x: 0 = drizzle(new Client());
export const m: 0 = drizzle.mock();
