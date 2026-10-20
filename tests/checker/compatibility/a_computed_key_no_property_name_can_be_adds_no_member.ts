// A computed key typed `boolean` (TS2464) names no member, so the literal
// has no excess property for it.
declare const flag: boolean;
declare const maybe: string | undefined;
export const keyed: { a: number } = { [flag]: 1, a: 1 };
export const optional: { a: number } = { [maybe]: 1, a: 1 };
