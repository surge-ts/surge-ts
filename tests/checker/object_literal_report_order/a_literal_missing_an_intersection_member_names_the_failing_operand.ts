// @surge-compare: messages
// A fresh literal missing a member of an intersection target is reported
// against the first written operand it fails, beneath the head.
type Pair = { a: number; b: number } & { note?: string };
const empty: Pair = {};
const half: Pair = { a: 1 };
const either: Pair | string = { a: 1 };
declare function take(p: Pair): void;
take({ b: 2 });
