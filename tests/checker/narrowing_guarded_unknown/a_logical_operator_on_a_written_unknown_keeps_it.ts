// @strict: true
// A written `unknown` is its own definitely-falsy part, so `u && x` is
// `unknown`; `u || x` keeps its non-nullable `{}` beside `x`.
declare const value: unknown;
export const b: boolean = value && typeof value === "object";
export const c: string = value && "x";

export function f(v: unknown, o: { p: unknown }) {
  const d: boolean = v && typeof v === "object";
  const e: number = v || 1;
  const g: string = o.p && "y";
  const h: unknown = v && 1;
  const i: {} | null | undefined = v && 1;
  const j: {} = v || 1;
}
