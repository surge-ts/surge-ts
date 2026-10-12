// @strict: true
// @module: commonjs
// @allowJs: true
// @checkJs: true
// @filename: mod1.js
Object.defineProperty(exports, "setonlyAccessor", {
    /** @param {string} str */
    set(str) {
        this.rwAccessors = Number(str)
    }
});
Object.defineProperty(exports, "thing", { value: 42, writable: true });
// @filename: a.ts
const o = {};
Object.defineProperty(o, "x", { get() { return this.y; } });
export {};
