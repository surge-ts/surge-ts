// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: alias.js
exports.apply = undefined;
exports.apply = undefined;
function a() { }
exports.apply = a;
exports.apply()
exports.apply = 'ok'
var OK = exports.apply.toUpperCase()
exports.apply = 1
// @filename: test.js
const { apply } = require('./alias')
const result = apply.toFixed()
