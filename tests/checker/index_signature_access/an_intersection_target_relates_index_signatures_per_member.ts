// An intersection target is related member by member: one member's
// `[x: string]: any` does not excuse another member's `[x: number]: any`,
// which an interface without index signatures lacks (TS2322).
interface StringTo<T> { [x: string]: T; }
interface NumberTo<T> { [x: number]: T; }
interface StringAndNumberTo<T> extends StringTo<T>, NumberTo<T> { }
interface Obj { hello: string; world: number; }
declare let both: StringTo<any> & NumberTo<any>;
declare let merged: StringAndNumberTo<any>;
declare let obj: Obj;
both = obj;
merged = obj;
