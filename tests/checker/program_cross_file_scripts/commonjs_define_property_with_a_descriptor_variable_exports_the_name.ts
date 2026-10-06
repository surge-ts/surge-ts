// @allowJs: true
// @checkJs: true
// @module: commonjs
// @filename: mod1.js
const obj = { value: 42, writable: true };
Object.defineProperty(exports, "thing", obj);
const name = "other";
Object.defineProperty(exports, name, obj);
// @filename: importer.js
const mod = require("./mod1");
mod.thing;
mod.other;
mod.thing = 0;
