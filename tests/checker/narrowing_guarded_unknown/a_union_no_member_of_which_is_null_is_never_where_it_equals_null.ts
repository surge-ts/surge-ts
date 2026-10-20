// @strict: true
// Under `strictNullChecks`, a union none of whose members can be `null` is
// `never` where it equals `null` (`getAdjustedTypeWithFacts` with `EQNull`).
declare const t: {} | undefined;
export const viaTernary: 0 = t === null ? t : 0;
export function inIf(s: string | undefined) {
    if (s === null) {
        s.length;
    }
}
export const isDefined = (v: unknown) => v !== null;
declare const values: unknown[];
export const found: {} | undefined = values.find(isDefined);
