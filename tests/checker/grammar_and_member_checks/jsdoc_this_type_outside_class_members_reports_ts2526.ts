// @allowJs: true
// @checkJs: true
// @filename: a.js
/** @return {this} */
function g() { return this; }

/** @param {this} x */
function h(x) {}

const o = {
    /** @return {this} */
    m() { return this; },
    /** @type {this} */
    p: 1,
};

/** @type {this} */
var top = 1;

function Cp() {
    /** @type {this} */
    this.dit = this;
    /** @return {this} */
    this.m3 = () => this;
}
/** @return {this} */
Cp.prototype.m2 = function () { return this; };

class K {
    /** @return {this} */
    m() {
        /** @type {this} */
        const self = this;
        /** @return {this} */
        const f = () => self;
        return f();
    }
    /** @return {this} */
    static s() { return this; }
    /** @param {this} other */
    constructor(other) {
        /** @type {this} */
        const me = this;
    }
    /** @type {this} */
    field = this;
    /** @type {this} */
    static sfield = 1;
    /** @return {this is K} */
    is() { return true; }
}
