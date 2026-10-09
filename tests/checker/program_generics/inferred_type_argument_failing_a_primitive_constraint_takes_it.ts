// @strict: true
// @lib: es2023
function pick<T, U extends T>(x: T, y: U): U { return y; }
interface NumberVariant extends Number { x: number; }
declare const variant: NumberVariant;
const fromVariant = pick(1, variant);
const fromString = pick(1, "");
enum Flag { On = 1 }
const fromEnum = pick(1, Flag.On);
