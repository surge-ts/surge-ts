// @module: node16
// @resolveJsonModule: true
// @target: es2022
// @filename: config.json
{ "version": 1 }

// @filename: loosey.cts
import config2 from "./config.json";
import * as config3 from "./config.json";
config2.version;
config3.version;
config3.default;
