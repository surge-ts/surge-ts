// @allowJs: true
// @noEmit: true
// @target: esnext
// @module: esnext
// @jsx: preserve
// @filename: a.js
class A {
    #a;
    m() {
        this.#a;
        this.#b;
    }
}
function foo() {
    await new Promise(undefined);
}
if ({} === {}) {}
const n = 1;
n.missing;
// @filename: b.jsx
const first = 1, second = 2;
const element = <div className={first, second} />;
// @filename: c.js
if ({} == {}) {}
if ([] !== []) {}
