// @strict: true
// A written `unknown` is comparable to anything, so `u !== null` is a
// `boolean` like any other comparison; it was read as unknown, which hid
// these TS2322s.
declare const u: unknown;
function viaDeclaration(w: unknown) {
    return w !== null;
}
export const fromDeclaration: number = viaDeclaration(1);
export const viaArrow = (w: unknown) => w !== null;
export const fromArrow: number = viaArrow(1);
export const direct = u !== null;
export const fromDirect: number = direct;
export const looseDirect: number = u != undefined;
