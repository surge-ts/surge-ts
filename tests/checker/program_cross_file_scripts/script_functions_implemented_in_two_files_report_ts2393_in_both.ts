// @allowJs: true
// @filename: a.ts
function foo() {
    return 30;
}
// @filename: c.ts
function foo() {
    return 10;
}
// @filename: b.js
function bar() {
    return 10;
}
// @filename: d.ts
function bar() {
    return 30;
}
