// @module: node16
// @target: es2022
// @resolveJsonModule: true
// @strict: true
// In Node.js a JSON module has no named exports: an ECMAScript module that
// imports one gets its default alone (TS1544 on a named import), and its
// namespace object holds nothing else.
// @filename: config.json
{
  "version": 1
}
// @filename: main.mts
import config from "./config.json";
import { version } from "./config.json";
import { default as config1 } from "./config.json";
import * as all from "./config.json";
all.version;
all.default.version;
config.version;
config1.version;
