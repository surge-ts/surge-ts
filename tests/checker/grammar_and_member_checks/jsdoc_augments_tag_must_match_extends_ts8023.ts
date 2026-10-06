// @allowJs: true
// @checkJs: true
// @filename: a.js
class A { constructor() { this.x = 0; } }
class B {}
const ns = { A };

/** @augments A */
class C extends B {}

/** @extends {Mismatch} */
class D extends ns.A {}

/** @augments */
class E extends A {}

/** @augments {ns.A} */
class F extends A {}

/**
 * @augments A
 * @augments B
 */
class G extends A {}

const H = /** @augments B */ class extends A {};
