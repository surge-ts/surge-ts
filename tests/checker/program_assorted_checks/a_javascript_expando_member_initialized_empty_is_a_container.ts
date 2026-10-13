// @allowJs: true
// @checkJs: true
// @strict: true
// @noEmit: true
// @filename: a.js
const foo = {};
foo["baz"] = {};
foo["baz"]["blah"] = 3;
const bar = {};
bar.baz = {};
bar.baz.blah = 3;
bar.baz.blah = 4;
/** @type {string} */
const s = bar.baz.blah;
const qux = {};
qux.baz = { a: 1 };
qux.baz.blah = 3;
