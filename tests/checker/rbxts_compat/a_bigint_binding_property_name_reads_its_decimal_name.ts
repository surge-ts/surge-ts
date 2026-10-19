// `getPropertyNameForPropertyNameNode`: a decimal bigint key names the
// property its text spells (`{ 0n: value }` reads `"0n"`), through a string
// index too; a hex one is indexed as the `bigint` it is (TS2538).
export {};
declare const obj: { [key: string]: number };
const { 0n: value } = obj;
const fromValue: string = value;
declare const pair: { "16": string; "16n": boolean; "0n": number };
const { 0n: zero } = pair;
const fromZero: string = zero;
const { 0x10n: sixteen } = pair;
