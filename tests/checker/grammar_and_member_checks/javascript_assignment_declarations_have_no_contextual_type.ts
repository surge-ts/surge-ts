// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: a.js
// A JavaScript write that declares a member (an expando on a container
// declared without a type, `this.x` inside an object-literal member) has no
// contextual type unless the write carries a JSDoc `@type`.
/** @typedef {{ status: 'done', m(n: number): void }} DoneStatus */
var ns = {};
/** @type {DoneStatus} */
ns.x = { status: 'done', m(n) { } };
ns.x = { status: 'done', m(n) { } };
/** @type {{ a(): void; b?(n: number): number; }} */
const typed = {
    a() {
        this.b = n => n;
    }
};
const untyped = {
    a() {
        this.c = n => n;
    }
};
