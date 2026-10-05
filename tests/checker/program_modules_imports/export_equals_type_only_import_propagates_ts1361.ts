// @module: commonjs
// @esModuleInterop: true
// @filename: a.ts
export class A {}
// @filename: b.ts
import type * as types from './a';
export = types;
// @filename: d.ts
import types from './b';
new types.A();
// @filename: e.ts
import types = require('./b');
new types.A();
// @filename: f.ts
import * as types from './b';
new types.A();
