// @allowJs: true
// @checkJs: true
// @target: es2022
// @filename: a.ts
class A { x; #y; z; constructor(v: number) { this.x = 1; this.#y = v; } }
class C2 { static prototype; static other; }
// @filename: b.js
class B { #y; #z; constructor() { this.#y = 2; } }
