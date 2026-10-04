declare const src: { a: number; b: string };
let computed = 'b';
const literal = 'a';
var { [computed]: stillNotGreat, ...rest } = src;
var { [literal]: fine } = src;
declare const dict: Record<string, number>;
var { [computed]: alsoFine } = dict;
