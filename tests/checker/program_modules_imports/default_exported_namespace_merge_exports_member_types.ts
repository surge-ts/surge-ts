// @strict: true
// @filename: m.ts
declare class ClientImpl { c: 1 }
declare const PG: { Client: typeof ClientImpl };
declare namespace PG {
  type Client = ClientImpl;
}
export default PG;
// @filename: a.ts
import pg from './m';
export const x: 0 = null! as pg.Client;
export const y: 0 = new pg.Client();
