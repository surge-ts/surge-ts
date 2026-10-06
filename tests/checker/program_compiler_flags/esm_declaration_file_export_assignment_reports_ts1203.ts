// @module: nodenext
// @filename: other.d.mts
declare function example(): 5;
export = example;
// @filename: legacy.d.cts
declare function legacy(): 5;
export = legacy;
// @filename: main.mts
import legacy = require("./legacy.cjs");
legacy();
