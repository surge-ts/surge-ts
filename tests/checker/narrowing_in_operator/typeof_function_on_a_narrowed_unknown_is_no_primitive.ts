// @strict: true
declare const isPgEnumSym: unique symbol;
export function isPgEnum(obj: unknown): boolean {
	return !!obj && typeof obj === 'function' && isPgEnumSym in obj;
}
