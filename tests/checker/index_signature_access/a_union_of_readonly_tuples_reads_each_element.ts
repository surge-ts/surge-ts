declare const u: readonly [9, 8] | readonly [9, 0];
declare const w: readonly [9, 0] | undefined;
const a: 7 = (u)[1];
const b: 7 = (w ?? [9, 8] as const)[1];
let c: 0 | 1 | 8 | 9;
[[, c] = [9, 0] as const] = [[9, 8] as const];
const d: 7 = c;
