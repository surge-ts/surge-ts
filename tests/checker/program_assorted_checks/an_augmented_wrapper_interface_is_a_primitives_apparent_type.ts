// @filename: globals.ts
interface Boolean { doStuff(): string; }
interface Number { doOther(): string; }
// @filename: use.ts
interface NotBoolean { doStuff(): string; }
declare let b: NotBoolean;
const flag = true;
b = flag;
const n = 1;
const other: { doOther(): string } = n;
const missing: { nope(): string } = n;
let back: boolean = b;
export {};
