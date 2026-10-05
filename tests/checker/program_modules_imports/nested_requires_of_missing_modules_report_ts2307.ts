// @allowJs: true
// @checkJs: true
// @module: commonjs
// @filename: index.js
const a = require("./missing1");
require("./missing2");
function f() { const b = require("./missing3"); }
(function(require){ const m = require("./missing4"); })(null);
