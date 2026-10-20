// @strict: true
// In a conditional expression, a unit literal compared with itself is `never`
// where the comparison fails, as in an `if`.
export function narrowed(v: 2, s: "a") {
    const viaNotEquals: 0 = v !== 2 ? v : 0;
    const viaNot: 0 = !(v === 2) ? v : 0;
    const viaString: 0 = s !== "a" ? s : 0;
    const kept: 0 = v === 2 ? v : 0;
    return [viaNotEquals, viaNot, viaString, kept];
}
