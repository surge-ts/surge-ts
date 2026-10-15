// @allowJs: true
// @checkJs: true
// @filename: def.js
function declared() {}
declared.own = 1;
var expression = function () {};
// @filename: usage.js
declared.extra = 1;
expression.other = 1;
