// @allowJs: true
// @checkJs: true
// @filename: a.js
export function abc() { return 5; }
module.exports = { abc };
module.exports.x = 1;
// @filename: b.js
function def() { return 5; }
module.exports = { def };
