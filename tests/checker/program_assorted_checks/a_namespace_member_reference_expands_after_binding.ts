// @strict: true
interface Box<U> { selfNum: Box<number>; v: U }
declare namespace PT { export const a: Box<string>; }
const wrongValue: string = PT.a.selfNum.v;
const wrongWhole: number = PT.a.selfNum;
