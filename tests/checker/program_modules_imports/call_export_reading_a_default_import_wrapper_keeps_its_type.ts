// @module: esnext
// @moduleResolution: bundler
// @esModuleInterop: true
// @filename: node_modules/pgx/package.json
{ "name": "pgx", "types": "index.d.ts", "exports": { ".": { "import": "./index.d.mts", "require": "./index.d.ts" } } }
// @filename: node_modules/pgx/index.d.ts
export class Client { connect(): void; }
// @filename: node_modules/pgx/index.d.mts
import * as pg from "./index.js";
declare const PG: { Client: typeof pg.Client };
export default PG;
// @filename: base.ts
export class Base<T> { select(): T { return null!; } }
// @filename: driver.ts
import { Base } from './base';
export function make<S = Record<string, never>, C = unknown>(client: C): Base<S> & { $client: C } { return null!; }
// @filename: db.ts
import pg from 'pgx';
import { make } from './driver';
export const db = make(new pg.Client());
// @filename: use.ts
import { db } from './db';
export const whole: 0 = db;
export const client: 0 = db.$client;
export const select: 0 = db.select;
