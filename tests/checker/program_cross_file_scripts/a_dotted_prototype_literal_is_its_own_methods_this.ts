// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: a.js
var Ns = {};
Ns.F = function () { this._map = {}; };
Ns.F.prototype = {
    get() { return this._map; },
    own() { return this.get; }
};
