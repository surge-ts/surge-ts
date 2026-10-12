// @allowJs: true
// @checkJs: true
// @noUnusedParameters: true
// @target: es2022
// @filename: a.js
/** @template T */
function f() {}
/**
 * @template T
 * @param {T} x
 */
function used(x) { return x; }
/**
 * @template T
 * @returns {T | undefined}
 */
function returned() { return undefined; }
/** @template _T */
function underscored() {}
/**
 * @template T
 * @template V
 */
class C1 {
    constructor() {
        /** @type {T} */
        this.p;
    }
}
/**
 * @template T,V
 */
class C2 {
    constructor() { }
}
/**
 * @template T
 */
class C3 {
    /** @param {T} value */
    set(value) {}
}
/**
 * @template T,V
 */
class C4 {
    /** @type {V | undefined} */
    v = undefined;
}
