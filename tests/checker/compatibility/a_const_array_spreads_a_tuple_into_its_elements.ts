// @strict: true
// `as const` on an array literal spreads a tuple into its own elements.
const tuple = [1, 2, 3] as const;
export const spread = [1, ...tuple] as const;
export const length: 4 = spread.length;
export const last: 3 = spread[3];
