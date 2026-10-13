// @allowJs: true
// @checkJs: true
// @filename: mod1.js

/** @typedef {number} Foo */
class Bar {}
// @filename: mod2.js
class Foo { }
const Bar = 3;
// @filename: c.ts
class Baz {}
interface Qux { a: number }
// @filename: mod3.js
/** @typedef {number} Baz */
/** @typedef {{ b: string }} Qux */
var q = 1;
