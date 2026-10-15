// @strict: true
declare function trailingOptional<T extends string[]>(args: [...T, number?]): T;
const twoStrings: number = trailingOptional(["foo", "bar"]);
const withNumber: number = trailingOptional(["foo", "bar", 1]);
const oneString: number = trailingOptional(["foo"]);
declare function trailing<T extends unknown[]>(args: [...T, number]): T;
const beforeNumber: number = trailing(["foo", 1]);
declare function leading<T extends unknown[]>(args: [string, ...T]): T;
const afterString: number = leading(["foo", 1, true]);
