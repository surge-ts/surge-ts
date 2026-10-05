// @isolatedModules: true
// @module: commonjs
// @filename: exportEqualsT.ts
declare type T = number;
export = T;
// @filename: exportEqualsC.ts
declare class C {}
export = C;
// @filename: user.ts
export import T2 = require("./exportEqualsT");
export import C2 = require("./exportEqualsC");
import T3 = require("./exportEqualsT");
