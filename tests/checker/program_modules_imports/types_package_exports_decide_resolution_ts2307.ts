// @module: nodenext
// @filename: node_modules/@types/dedent/package.json
{ "name": "@types/dedent", "version": "1.0.0", "main": "" }
// @filename: node_modules/@types/dedent/index.d.ts
export declare function dedent(): void;
// @filename: node_modules/@types/dedent3/package.json
{ "name": "@types/dedent3", "version": "1.0.0", "main": "asdfasdfasdf", "exports": null }
// @filename: node_modules/@types/dedent3/index.d.ts
export declare function dedent3(): void;
// @filename: node_modules/@types/dedent4/package.json
{ "name": "@types/dedent4", "version": "1.0.0", "main": "asdfasdfasdf", "exports": "./asdfasdfasdf" }
// @filename: node_modules/@types/dedent4/index.d.ts
export declare function dedent4(): void;
// @filename: node_modules/@types/dedent5/package.json
{ "name": "@types/dedent5", "version": "1.0.0", "exports": { ".": { "types": "./main.d.ts" } } }
// @filename: node_modules/@types/dedent5/main.d.ts
export declare function dedent5(): void;
// @filename: index.cts
import { dedent } from "dedent";
import { dedent3 } from "dedent3";
import { dedent4 } from "dedent4";
import { dedent5 } from "dedent5";
// @filename: node_modules/nopkg/index.d.ts
export declare function nopkg(): void;
// @filename: node_modules/@types/nopkgtypes/index.d.ts
export declare function nopkgtypes(): void;
// @filename: node_modules/withpkg/package.json
{ "name": "withpkg", "version": "1.0.0" }
// @filename: node_modules/withpkg/index.d.ts
export declare function withpkg(): void;
// @filename: index.mts
import { dedent } from "dedent";
import { dedent4 } from "dedent4";
import { nopkg } from "nopkg";
import { nopkgtypes } from "nopkgtypes";
import { withpkg } from "withpkg";
// @filename: other.cts
import { nopkg } from "nopkg";
import { nopkgtypes } from "nopkgtypes";
