// `x instanceof o` with an object type's own `[Symbol.hasInstance]` method is
// the call `o[Symbol.hasInstance](x)`: its parameter has to take the left
// operand (TS2860) and it has to return a boolean (TS2861).
declare const takesX: { [Symbol.hasInstance](value: { x: number }): boolean };
declare const hasY: { y: string };
declare const hasX: { x: number };
export const rejected = hasY instanceof takesX;
export const accepted = hasX instanceof takesX;

declare const numeric: { [Symbol.hasInstance](value: unknown): number };
export const notBoolean = hasX instanceof numeric;

declare const guard: { [Symbol.hasInstance](value: unknown): value is { x: number } };
export const predicate = hasY instanceof guard;

class Plain {}
export const constructor = hasY instanceof Plain;
