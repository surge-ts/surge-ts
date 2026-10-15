// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: a.js
/** @type {{ d(): void; e?(n: number): number; f?(n: number): number; g?: number }} */
const o2 = {
    d() {
        this.e = this.f = m => this.g || m;
        const z = (this.f = p => p);
        this.f = (k) => k;
    }
};
