// @allowJs: true
// @checkJs: true
// @strict: false
// @filename: index.js
// tsgo has no JavaScript constructor functions: `new` on a plain function
// that does not return void is TS2350, and inside its own body the return
// type is still being read (circular, so the error type).
function C1() {
    if (!(this instanceof C1)) return new C1();
    this.x = 1;
}
var c1 = new C1();
var C2 = function () {
    if (!(this instanceof C2)) return new C2();
    this.x = 1;
};
var c2 = new C2();
