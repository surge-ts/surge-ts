// @allowJs: true
// @checkJs: true
// @module: commonjs
// @target: es2022
// @filename: mod.js
module.exports = {
    Baz: class { }
};
module.exports = {
    Quack: 2
};
// @filename: use.js
const mod = require("./mod.js");
new mod.Baz();
mod.Quack.toFixed();
