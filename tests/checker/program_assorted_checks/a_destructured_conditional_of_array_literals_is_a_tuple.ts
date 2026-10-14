// @strict: true
// @noUncheckedIndexedAccess: true
// The binding pattern is each branch's contextual type, so both array
// literals are tuples and their elements are never `undefined`.
declare const fit: number;
declare const w: number, h: number;
export function scale(): number {
  const [x, y] = fit > 0 ? [fit, fit] : [w, h];
  return x * y;
}
export function nested(sector: number): number {
  const [r, g, b] = sector < 1 ? [1, 2, 0] : sector < 2 ? [2, 1, 0] : [0, 1, 2];
  return r + g + b;
}
export function unpatterned(): number {
  const pair = fit > 0 ? [fit, fit] : [w, h];
  return pair[0];
}
