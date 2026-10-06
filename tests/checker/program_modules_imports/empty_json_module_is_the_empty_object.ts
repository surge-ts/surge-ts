// @module: commonjs
// @resolveJsonModule: true
// @filename: b.json

// @filename: c.json
{"a": 1}
// @filename: file1.ts
import b1 = require('./b.json');
let x = b1.a;
import c1 = require('./c.json');
let y = c1.a;
