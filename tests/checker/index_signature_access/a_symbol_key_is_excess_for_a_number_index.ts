// A unique-symbol computed key names a property no number index signature
// covers (TS2353); a string index accepts it.
const sym = Symbol();
const o1: { [key: number]: string } = { [sym]: 'hello' };
const o2: { [key: string]: string } = { [sym]: 'hello' };
const o3: { [key: number]: string } = { [Symbol.iterator]: 'hello' };
declare const key: string;
const o4: { [key: number]: string } = { [key]: 'hello' };
