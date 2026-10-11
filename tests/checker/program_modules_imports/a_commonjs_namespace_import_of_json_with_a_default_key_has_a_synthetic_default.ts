// @module: node16
// @resolveJsonModule: true
// @target: es2022
// @filename: package.json
{ "name": "pkg", "version": "0.0.1", "default": "misdirection" }

// @filename: index.cts
import * as ns from "./package.json";
ns.name;
ns.default.name;
ns.default.version;
ns.default.toUpperCase();
