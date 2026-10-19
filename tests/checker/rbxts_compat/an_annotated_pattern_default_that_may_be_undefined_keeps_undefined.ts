// `getBindingElementTypeFromParentType`: under an annotation a defaulted
// element sheds `undefined` only for a default that cannot be `undefined`.
export {};
declare const values: number[];
declare function maybe(): number | undefined;
export function read({ value = maybe(), ...rest }: { value?: number; other: number }) {
	return [value, rest.other];
}
export function readArray([value = maybe()]: [number?]) {
	return value;
}
export function wrong({ value = "x" }: { value?: number }) {
	return value;
}
export function wrongMaybe({ value = maybe() as string | undefined }: { value?: number }) {
	return value;
}
