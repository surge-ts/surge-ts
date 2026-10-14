// @strict: true
// Each branch of a returned conditional is related to the return type as
// written: `readonly T[]` accepts a readonly branch.
type Issue = { message: string };
declare function other(): readonly Issue[];
declare const c: boolean;
export function a1(): readonly Issue[] {
  return c ? [] : other();
}
export function a2(): readonly Issue[] {
  return c ? other() : other();
}
export function a3(): Issue[] {
  return c ? [] : other();
}
