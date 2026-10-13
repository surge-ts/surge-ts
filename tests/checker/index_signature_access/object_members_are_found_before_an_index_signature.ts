// `getPropertyOfType` finds the global `Object` members before any index
// signature, so `hasOwnProperty` on a record is the method, not its value type.
declare const record: Record<string, unknown>;
if (record.hasOwnProperty("isPrototypeOf") === false) {}
declare const numbers: Record<string, number>;
const own: string = numbers.hasOwnProperty("a");
const text: number = numbers.toString;
