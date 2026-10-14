// @strict: true
// An array literal whose contextual type is a union with a tuple member is a
// tuple, whether the member has a rest slot or the literal is the right
// operand of `??`.
declare const caps: string[];
declare const i: number;
export function find(): [] | [number, number, ...string[]] {
  if (i > 1) return [i, i, ...caps];
  return i >= 0 ? [i + 1, i] : [];
}
export function next(entries: [unknown, unknown][]): [unknown, unknown] | [undefined] {
  const first = entries[0];
  return first ?? [undefined];
}
export const wrong: [] | [number, number, ...string[]] = [1, 2, 3];
