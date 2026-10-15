// @declaration: true
// @allowJs: true
// @checkJs: true
// @noEmit: true
// @lib: es6
// @filename: mod.d.ts
interface Item { x: string }
declare function getItems(): Item[];
export = getItems;
// @filename: index.js
const items = require("./mod")();
module.exports = items;
