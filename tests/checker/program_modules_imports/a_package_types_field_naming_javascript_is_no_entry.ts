// @module: commonjs
// A package's `types` field serves declarations only: one naming a JavaScript
// file with no declaration beside it gives no entry, and without a `main` or
// an `index.js` the side-effect import resolves nowhere (TS2882). A `main`
// naming JavaScript still resolves.
// @filename: node_modules/foo/package.json
{ "types": "foo.js" }
// @filename: node_modules/foo/foo.js
module.exports = 1;
// @filename: node_modules/bar/package.json
{ "main": "index.js" }
// @filename: node_modules/bar/index.js
module.exports = 1;
// @filename: a.ts
import "foo";
import "bar";
export {};
