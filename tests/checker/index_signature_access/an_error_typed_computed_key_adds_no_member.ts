// @target: es2015
var obj = { [Symbol.nonsense]: 0 };
obj = {};
const s: string = obj;
