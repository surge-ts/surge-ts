// @strict: true
// A written `unknown` element absorbs the union of an array literal's
// elements, so the literal is `unknown[]`, not an untyped array.
declare const u: unknown;
const mixed = [u, 1];
export const fromMixed: number = mixed;
export function inBody<T>(t: T) {
    const pair = [t, u];
    const n: number = pair;
    return n;
}
