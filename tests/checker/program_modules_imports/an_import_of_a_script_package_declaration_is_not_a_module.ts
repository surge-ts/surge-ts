// A package's declaration file with no import or export is a script: binding
// an import to it is TS2306, as for any program file that is no module.
// @filename: node_modules/@types/react/package.json
{ "name": "@types/react", "version": "16.4.6" }
// @filename: node_modules/@types/react/index.d.ts
declare global { }
// @filename: index.ts
import * as React from 'react';
export var x = 1;
