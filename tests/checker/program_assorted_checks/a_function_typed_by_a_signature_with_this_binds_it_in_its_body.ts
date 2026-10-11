// @allowJs: true
// @checkJs: true
// @filename: types.d.ts
export interface Foo { foo: () => void; }
export type M = (this: Foo) => void;
// @filename: a.js
/** @type {import('./types').M} */
export function f2() {
    this.foo();
    this.test();
}
