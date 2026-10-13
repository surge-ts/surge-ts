// @allowJs: true
// @checkJs: true
// @filename: a.js
exports.a = undefined;
// @filename: b.js
exports.b = exports.c = undefined;
// @filename: c.js
exports.d = exports.e = exports.f = void 0;
exports.d = 1;
