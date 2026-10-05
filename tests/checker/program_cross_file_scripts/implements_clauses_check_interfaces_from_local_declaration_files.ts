// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: defs.d.ts
interface A {
    mNumber(): number;
}
// @filename: a.js
/** @implements A */
class B3 {
}
/** @implements {A} */
class B2 {
    mNumber() {
        return "";
    }
}
// @filename: c.ts
class C3 implements A {}
