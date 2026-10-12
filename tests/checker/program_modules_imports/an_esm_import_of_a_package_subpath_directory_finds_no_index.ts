// @module: node16
// @noUncheckedSideEffectImports: true
// @strict: true
// @target: es2022
// @filename: package.json
{ "type": "module" }
// @filename: node_modules/i-have-a-dir-and-main/package.json
{ "name": "i-have-a-dir-and-main", "version": "1.0.0", "type": "module", "main": "dist/index.js" }
// @filename: node_modules/i-have-a-dir-and-main/dist/index.d.ts
export declare const a = 1;
// @filename: node_modules/i-have-a-dir-and-main/dist/dir/index.d.ts
export declare const b = 2;
// @filename: index.ts
import 'i-have-a-dir-and-main';
import 'i-have-a-dir-and-main/dist/dir';
