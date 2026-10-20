// @strict: true
// @lib: es2022
// tsc's `chooseOverload` infers a candidate resolved with its type
// parameter's default (`Object.fromEntries<T = any>`) from value arguments
// too, with no contextual return.
declare const entries: [string, number][];
const record = Object.fromEntries(entries);
export const wrong: string = record.x;
declare const pairs: Map<string, boolean>;
const flags = Object.fromEntries(pairs);
export const alsoWrong: number = flags.x;
