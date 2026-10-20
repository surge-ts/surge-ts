// @strict: true
// tsc removes `undefined` from a defaulted parameter in its body only when
// the initializer cannot be `undefined` itself.
declare const known: number | undefined;
export function fromMaybe(value = known) {
    const n: number = value;
    return n;
}
export const arrow = (value = known) => {
    const n: number = value;
    return n;
};
export function fromLiteral(value = 1) {
    const n: number = value;
    return n;
}
// A required parameter after it puts `undefined` in the signature, which the
// initializer still fills.
export function leading(cursor = 0, ts: number) {
    const n: number = cursor;
    return n + ts;
}
export const leadingArrow = (cursor = 0, ts: number) => {
    const n: number = cursor;
    return n + ts;
};
