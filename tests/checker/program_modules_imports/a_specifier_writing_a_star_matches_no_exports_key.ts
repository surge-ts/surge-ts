// @module: node18
// @target: es2022
// @filename: node_modules/double-asterisk/package.json
{ "name": "double-asterisk", "version": "1.0.0", "type": "module", "exports": { "./a/*/b/*/c/*": "./example.js", "./plain": "./example.js" } }
// @filename: node_modules/double-asterisk/example.d.ts
export {};
// @filename: main.mts
import {} from "double-asterisk/a/*/b/*/c/*";
import {} from "double-asterisk/plain";
