// @allowJs: true
// @checkJs: true
// @module: esnext
// @moduleResolution: bundler
// @filename: mod.js
/** @typedef {number} T */
module.exports.n = {};
module.exports.Classic = class {};
// @filename: use.js
import * as s from './mod'
/** @param {s.n.K} c
    @param {s.Classic} classic
    @param {s.Missing} m
    @param {s.T} t */
function f(c, classic, m, t) {}
// @filename: use2.ts
import * as s from './mod';
let a: s.n.K;
let b: s.Missing;
let c: s.T = 1;
export {};
